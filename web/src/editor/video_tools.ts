export interface VideoElementProps {
  file_path: string;
  in_point_s?: number;
  out_point_s?: number;
  loop_playback?: boolean;
  is_muted?: boolean;
  volume?: number;
  opacity?: number;
}

export function renderVideoDOM(containerEl: HTMLElement, props: VideoElementProps): void {
  // See note in shape_library.ts: don't touch containerEl's width/height here,
  // the caller already sizes it absolutely in px.
  containerEl.innerHTML = '';
  containerEl.style.overflow = 'hidden';
  containerEl.style.position = 'relative';

  const video = document.createElement('video');
  video.src = props.file_path;
  video.style.width = '100%';
  video.style.height = '100%';
  video.style.objectFit = 'cover';
  video.style.opacity = `${props.opacity ?? 1.0}`;
  video.loop = props.loop_playback ?? true;
  video.muted = props.is_muted ?? true;
  video.volume = props.volume ?? 1.0;
  video.playsInline = true;

  if (props.in_point_s !== undefined && props.in_point_s > 0) {
    video.currentTime = props.in_point_s;
  }

  // Handle in/out loop clamping
  video.addEventListener('timeupdate', () => {
    if (props.out_point_s !== undefined && video.currentTime >= props.out_point_s) {
      if (props.loop_playback !== false) {
        video.currentTime = props.in_point_s ?? 0;
        video.play().catch(() => {});
      } else {
        video.pause();
      }
    }
  });

  containerEl.appendChild(video);
}
