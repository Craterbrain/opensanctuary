// Frame-accurate, hardware-decoded video/audio playback for the Live Output
// display, replacing <video>'s playbackRate-slew/hard-seek drift correction
// (see TimecodeSyncEngine in live_output.ts) with direct frame selection and
// sample-accurate Web Audio scheduling -- no pitch bending, no seek judder.
//
// Built on `mediabunny` (MPL-2.0, https://mediabunny.dev): its UrlSource
// streams via HTTP Range requests (matching /media's existing ServeDir range
// support), and its CanvasSink/AudioBufferSink give timestamp-indexed access
// to hardware-decoded frames/audio via WebCodecs internally. This module
// never talks to VideoDecoder/AudioDecoder directly -- mediabunny owns that.
//
// This is tier 2 of the three-tier renderer selection in live_output.ts:
// window.AndroidTV bridge (native ExoPlayer) > WebCodecsPlayer (this file) >
// plain <video> + TimecodeSyncEngine (unchanged fallback for unsupported
// browsers or files this pipeline can't open).
import {
  Input,
  ALL_FORMATS,
  UrlSource,
  CanvasSink,
  AudioBufferSink,
  type InputVideoTrack,
  type InputAudioTrack,
} from 'mediabunny';
import { clock } from './timesync';

/** Matches video_sync.ts's tier-3 hard-seek threshold, for behavioral parity. */
const HARD_SEEK_THRESHOLD_SEC = 0.5;

function sleep(ms: number): Promise<void> {
  if (ms <= 0) return Promise.resolve();
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export function isWebCodecsSupported(): boolean {
  return typeof (window as any).VideoDecoder !== 'undefined' && typeof (window as any).AudioDecoder !== 'undefined';
}

/** Mirrors TimecodeSyncEngine's masterState shape, pre-resolved into the
 * anchor math it already computes in calculateTargetTime -- the caller does
 * that resolution once per snapshot; this class just needs the result. */
export interface WebCodecsSyncState {
  isPlaying: boolean;
  /** Playback position (seconds) corresponding to startAnchorEpochMs, or the
   * paused/preroll position to hold when isPlaying is false. */
  baseTimeSec: number;
  /** Server-epoch ms instant at which playback began from baseTimeSec. */
  startAnchorEpochMs: number;
  /** True if startAnchorEpochMs is still in the future (scheduled start). */
  isFutureScheduled: boolean;
  loop: boolean;
  muted: boolean;
  volume: number;
}

interface Anchor {
  baseTimeSec: number;
  startAnchorEpochMs: number;
  loop: boolean;
}

function anchorsEqual(a: Anchor | null, b: Anchor): boolean {
  return !!a && a.baseTimeSec === b.baseTimeSec && a.startAnchorEpochMs === b.startAnchorEpochMs && a.loop === b.loop;
}

/** Quick, disposable probe: can this pipeline actually open and decode the
 * given URL in this browser? Used to decide whether to use WebCodecsPlayer
 * or fall back to <video> for a specific file, since codec/container support
 * varies by browser even when WebCodecs itself exists. */
export async function canPlayWithWebCodecs(url: string): Promise<boolean> {
  if (!isWebCodecsSupported()) return false;
  let input: Input | null = null;
  try {
    input = new Input({ source: new UrlSource(url), formats: ALL_FORMATS });
    const videoTrack = await input.getPrimaryVideoTrack();
    if (!videoTrack) return false;
    return await videoTrack.canDecode();
  } catch {
    return false;
  } finally {
    input?.dispose();
  }
}

export class WebCodecsPlayer {
  private canvas: HTMLCanvasElement;
  private ctx: CanvasRenderingContext2D;
  private input: Input;
  private videoTrack: InputVideoTrack;
  private audioTrack: InputAudioTrack | null;
  private canvasSink: CanvasSink;
  private audioSink: AudioBufferSink | null;
  private audioCtx: AudioContext | null = null;
  private gainNode: GainNode | null = null;
  private scheduledSources: AudioBufferSourceNode[] = [];

  private durationSec = 0;
  private playGeneration = 0;
  private lastAnchor: Anchor | null = null;
  private scheduledStartTimer: ReturnType<typeof setTimeout> | null = null;
  private muted = true;
  private volume = 1.0;
  private _currentTimeSec = 0;
  private onEndedCb: (() => void) | null = null;

  private constructor(
    canvas: HTMLCanvasElement,
    input: Input,
    videoTrack: InputVideoTrack,
    audioTrack: InputAudioTrack | null,
    canvasSink: CanvasSink,
    audioSink: AudioBufferSink | null,
  ) {
    this.canvas = canvas;
    const ctx = canvas.getContext('2d');
    if (!ctx) throw new Error('2D canvas context unavailable');
    this.ctx = ctx;
    this.input = input;
    this.videoTrack = videoTrack;
    this.audioTrack = audioTrack;
    this.canvasSink = canvasSink;
    this.audioSink = audioSink;
  }

  static isSupported = isWebCodecsSupported;

  /** Opens `url` and prepares decode sinks. Throws on anything unsupported
   * so the caller can fall back to <video> -- never partially succeeds. */
  static async create(
    canvas: HTMLCanvasElement,
    url: string,
    opts: { fit: 'contain' | 'cover'; onEnded?: () => void },
  ): Promise<WebCodecsPlayer> {
    if (!isWebCodecsSupported()) throw new Error('WebCodecs not supported in this browser');

    const input = new Input({ source: new UrlSource(url), formats: ALL_FORMATS });
    const videoTrack = await input.getPrimaryVideoTrack();
    if (!videoTrack) throw new Error('No video track in file');
    if (!(await videoTrack.canDecode())) throw new Error('Video codec not decodable via WebCodecs here');

    let audioTrack = await input.getPrimaryAudioTrack();
    if (audioTrack && !(await audioTrack.canDecode())) {
      audioTrack = null; // play video-only rather than failing the whole file over audio
    }

    const canvasSink = new CanvasSink(videoTrack, {
      width: canvas.width,
      height: canvas.height,
      fit: opts.fit,
      poolSize: 2,
    });
    const audioSink = audioTrack ? new AudioBufferSink(audioTrack) : null;

    const player = new WebCodecsPlayer(canvas, input, videoTrack, audioTrack, canvasSink, audioSink);
    player.onEndedCb = opts.onEnded ?? null;
    player.durationSec = await input.computeDuration([videoTrack, ...(audioTrack ? [audioTrack] : [])]);

    if (audioSink) {
      player.audioCtx = new AudioContext();
      player.gainNode = player.audioCtx.createGain();
      player.gainNode.connect(player.audioCtx.destination);
      player.applyGain();
    }

    // Draw the first frame immediately so the canvas isn't blank before the
    // first applySyncState call arrives.
    await player.drawStaticFrame(0);

    return player;
  }

  get duration(): number {
    return this.durationSec;
  }

  get currentTime(): number {
    return this._currentTimeSec;
  }

  /** Needed because AudioContext can only start/resume from a real user
   * gesture -- live_output.ts's existing click/keydown unmute handlers call
   * this the same way they already unmute the legacy <video>/<audio>. */
  resumeAudioContext(): void {
    if (this.audioCtx && this.audioCtx.state === 'suspended') {
      this.audioCtx.resume().catch(() => {});
    }
  }

  setMuted(muted: boolean): void {
    this.muted = muted;
    this.applyGain();
  }

  setVolume(volume: number): void {
    this.volume = volume;
    this.applyGain();
  }

  private applyGain(): void {
    if (this.gainNode) {
      this.gainNode.gain.value = this.muted ? 0 : this.volume;
    }
  }

  /** The single entry point TimecodeSyncEngine-equivalent logic in
   * live_output.ts drives this class through -- analogous to
   * TimecodeSyncEngine.syncElement()'s video branch, but frame-exact instead
   * of playbackRate-based. Safe to call repeatedly with the same state
   * (idempotent no-op once the matching loop is already running). */
  applySyncState(state: WebCodecsSyncState | null): void {
    if (!state) {
      this.stop();
      return;
    }

    this.setMuted(state.muted);
    this.setVolume(state.volume);

    if (this.scheduledStartTimer) {
      clearTimeout(this.scheduledStartTimer);
      this.scheduledStartTimer = null;
    }

    if (!state.isPlaying || state.isFutureScheduled) {
      this.stopLoops();
      this.drawStaticFrame(state.baseTimeSec).catch(() => {});
      if (state.isFutureScheduled) {
        const delayMs = Math.max(0, state.startAnchorEpochMs - clock.getServerTimeMs());
        this.scheduledStartTimer = setTimeout(() => this.applySyncState(state), delayMs);
      }
      return;
    }

    const anchor: Anchor = {
      baseTimeSec: state.baseTimeSec,
      startAnchorEpochMs: state.startAnchorEpochMs,
      loop: state.loop,
    };
    if (anchorsEqual(this.lastAnchor, anchor) && this.playGeneration > 0) {
      return; // already running the right thing -- self-correcting internally, no restart needed
    }
    this.lastAnchor = anchor;
    this.startLoops(anchor);
  }

  private trueElapsed(anchor: Anchor): number {
    return anchor.baseTimeSec + Math.max(0, (clock.getServerTimeMs() - anchor.startAnchorEpochMs) / 1000);
  }

  private expectedTime(anchor: Anchor): number {
    const elapsed = this.trueElapsed(anchor);
    if (anchor.loop && this.durationSec > 0) {
      return elapsed % this.durationSec;
    }
    return Math.min(elapsed, this.durationSec || elapsed);
  }

  private async drawStaticFrame(timeSec: number): Promise<void> {
    try {
      const wrapped = await this.canvasSink.getCanvas(timeSec);
      if (wrapped) {
        this.ctx.drawImage(wrapped.canvas, 0, 0);
        this._currentTimeSec = wrapped.timestamp;
      }
    } catch {
      // Best-effort -- a stale preview frame is harmless.
    }
  }

  private startLoops(anchor: Anchor): void {
    this.playGeneration++;
    const gen = this.playGeneration;
    this.stopScheduledAudio();
    void this.runVideoLoop(gen, anchor);
    if (this.audioSink && this.audioCtx) {
      void this.runAudioLoop(gen, anchor);
    }
  }

  private stopLoops(): void {
    this.playGeneration++; // invalidates any in-flight loop iterations
    this.stopScheduledAudio();
  }

  private stopScheduledAudio(): void {
    for (const src of this.scheduledSources) {
      try {
        src.onended = null;
        src.stop();
      } catch {
        // already stopped/ended
      }
    }
    this.scheduledSources = [];
  }

  private stop(): void {
    this.stopLoops();
    this.lastAnchor = null;
    if (this.scheduledStartTimer) {
      clearTimeout(this.scheduledStartTimer);
      this.scheduledStartTimer = null;
    }
  }

  private async runVideoLoop(gen: number, anchor: Anchor): Promise<void> {
    while (gen === this.playGeneration) {
      const startAt = this.expectedTime(anchor);
      const iter = this.canvasSink.canvases(startAt);
      let brokeForReseek = false;

      for await (const frame of iter) {
        if (gen !== this.playGeneration) return;

        const target = this.expectedTime(anchor);
        const waitMs = (frame.timestamp - target) * 1000;

        if (waitMs < -HARD_SEEK_THRESHOLD_SEC * 1000) {
          brokeForReseek = true;
          break; // decode fell too far behind real time -- jump forward instead of catching up frame-by-frame
        }
        if (waitMs > 0) {
          await sleep(waitMs);
          if (gen !== this.playGeneration) return;
        }

        this.ctx.drawImage(frame.canvas, 0, 0);
        this._currentTimeSec = frame.timestamp;
      }

      if (gen !== this.playGeneration) return;
      if (brokeForReseek) continue;

      // Iterator ran out naturally: end of track.
      if (anchor.loop) {
        continue; // expectedTime() already wraps via modulo once wall-clock passes duration
      }
      this.onEndedCb?.();
      return;
    }
  }

  private async runAudioLoop(gen: number, anchor: Anchor): Promise<void> {
    const audioSink = this.audioSink;
    const audioCtx = this.audioCtx;
    if (!audioSink || !audioCtx) return;

    // Maps this class's "unwrapped" (never-modulo'd) media time to AudioContext time.
    const epochOffset = audioCtx.currentTime - this.trueElapsed(anchor);

    let loopBaseSec = this.trueElapsed(anchor) - this.expectedTime(anchor);

    while (gen === this.playGeneration) {
      const startAt = this.expectedTime(anchor);
      const iter = audioSink.buffers(startAt);

      for await (const chunk of iter) {
        if (gen !== this.playGeneration) return;

        const unwrappedChunkTime = loopBaseSec + chunk.timestamp;
        const startCtxTime = Math.max(audioCtx.currentTime + 0.02, unwrappedChunkTime + epochOffset);

        const src = audioCtx.createBufferSource();
        src.buffer = chunk.buffer;
        src.connect(this.gainNode!);
        src.start(startCtxTime);
        this.scheduledSources.push(src);
        src.onended = () => {
          this.scheduledSources = this.scheduledSources.filter((s) => s !== src);
        };

        // Don't schedule arbitrarily far ahead -- keep roughly a 2s lookahead window.
        const aheadSec = startCtxTime - audioCtx.currentTime;
        if (aheadSec > 2) {
          await sleep((aheadSec - 1) * 1000);
          if (gen !== this.playGeneration) return;
        }
      }

      if (gen !== this.playGeneration) return;
      if (!anchor.loop) return; // end of track, not looping -- video loop already fires onEnded

      // Looped: advance loopBaseSec by one full track duration so the next
      // iteration's (small, wrapped) chunk timestamps map to the correct,
      // ever-increasing AudioContext time instead of repeating the first
      // loop's schedule.
      loopBaseSec += this.durationSec;
    }
  }

  dispose(): void {
    this.stop();
    this.input.dispose();
    if (this.audioCtx) {
      this.audioCtx.close().catch(() => {});
      this.audioCtx = null;
    }
  }
}
