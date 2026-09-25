/**
 * OpenSanctuary / OS-Next Media Sync Controller
 * High-Precision Direct Inter-Window Synchronization Channel (<1ms latency)
 * BroadcastChannel('opensanctuary_media_sync')
 */

export interface MediaSyncPayload {
  type: 'MEDIA_SYNC';
  action: 'play' | 'pause' | 'seek' | 'stop' | 'pulse' | 'loop' | 'close';
  currentTime: number;
  isPlaying: boolean;
  isLooping: boolean;
  timestamp: number;
  start_at_epoch_ms?: number | null;
}

export class MediaSyncManager {
  private channel: BroadcastChannel | null = null;
  private heartbeatTimer: any = null;
  private isConnectedToLiveOutput: boolean = false;
  private isLooping: boolean = false;
  private getVideoElCallback: () => HTMLVideoElement | null;

  constructor(getVideoEl: () => HTMLVideoElement | null) {
    this.getVideoElCallback = getVideoEl;
    try {
      this.channel = new BroadcastChannel('opensanctuary_media_sync');
      this.channel.addEventListener('message', (e) => {
        if (e.data?.action === 'output_ready') this.isConnectedToLiveOutput = true;
        if (e.data?.action === 'output_closed') this.isConnectedToLiveOutput = false;
      });
    } catch (e) {
      console.warn('[MediaSync] BroadcastChannel not available in this environment:', e);
    }

    if (typeof window !== 'undefined') {
      window.addEventListener('beforeunload', () => {
        this.broadcastSync('close', 0, false, false);
      });
    }
  }

  setLooping(looping: boolean) {
    this.isLooping = looping;
  }

  isLiveConnected(): boolean {
    return this.isConnectedToLiveOutput;
  }

  broadcastSync(
    action: MediaSyncPayload['action'],
    currentTime?: number,
    isPlaying?: boolean,
    isLooping?: boolean,
    executeAtEpoch?: number | null
  ) {
    const videoEl = this.getVideoElCallback();
    const curTime = currentTime !== undefined ? currentTime : (videoEl ? videoEl.currentTime : 0);
    const playing = isPlaying !== undefined ? isPlaying : (videoEl ? !videoEl.paused : false);
    const looping = isLooping !== undefined ? isLooping : this.isLooping;

    const payload: MediaSyncPayload = {
      type: 'MEDIA_SYNC',
      action,
      currentTime: curTime,
      isPlaying: playing,
      isLooping: looping,
      timestamp: Date.now(),
      start_at_epoch_ms: executeAtEpoch || null,
    };

    if (this.channel) {
      try {
        this.channel.postMessage(payload);
      } catch (err) {
        console.warn('[MediaSync] Error posting message:', err);
      }
    }
  }

  startHeartbeat(getCurrentTime?: () => number) {
    this.stopHeartbeat();
    this.heartbeatTimer = setInterval(() => {
      const videoEl = this.getVideoElCallback();
      const isPlaying = videoEl && !videoEl.paused && videoEl.style.display !== 'none';
      if (!isPlaying) {
        this.stopHeartbeat();
        return;
      }
      if (this.isConnectedToLiveOutput) {
        const time = getCurrentTime ? getCurrentTime() : (videoEl ? videoEl.currentTime : 0);
        this.broadcastSync('pulse', time, true, this.isLooping);
      }
    }, 500);
  }

  stopHeartbeat() {
    if (this.heartbeatTimer) {
      clearInterval(this.heartbeatTimer);
      this.heartbeatTimer = null;
    }
  }
}
