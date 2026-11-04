/**
 * Pure helpers for the mobile remote (remote_client.ts). Kept apart from that entry point, which
 * touches the DOM and starts pairing as soon as it loads, so they can be unit-tested directly.
 */

export type SwipeCommand = 'NextSlide' | 'PrevSlide' | 'NextItem' | 'PrevItem';

/**
 * What a finished touch does: a quick (< 450 ms), mostly-straight swipe of more than 45 px.
 * Left/right step slides in every view; up/down step items only in the prompter, because the
 * schedule and slide views scroll vertically.
 */
export function swipeCommand(deltaX: number, deltaY: number, elapsedMs: number, activeView: string): SwipeCommand | null {
  if (elapsedMs >= 450) return null;
  const absX = Math.abs(deltaX);
  const absY = Math.abs(deltaY);
  if (absX > 45 && absX > absY * 1.5) return deltaX < 0 ? 'NextSlide' : 'PrevSlide';
  if (activeView === 'view-prompter' && absY > 45 && absY > absX * 1.5) return deltaY < 0 ? 'NextItem' : 'PrevItem';
  return null;
}

export function getItemTypeIcon(type: string): string {
  switch ((type || '').toLowerCase()) {
    case 'song': return '🎵';
    case 'scripture': return '📖';
    case 'presentation': return '📽️';
    case 'media': return '🎬';
    case 'header': return '🏷️';
    default: return '📄';
  }
}

export function getTagClass(tag: string): string {
  if (!tag) return '';
  const t = tag.toUpperCase();
  if (t.startsWith('V')) return 'tag-v';
  if (t.startsWith('C')) return 'tag-c';
  if (t.startsWith('B')) return 'tag-b';
  if (t.startsWith('P')) return 'tag-p';
  if (t.startsWith('E')) return 'tag-e';
  if (t.startsWith('I')) return 'tag-i';
  return '';
}
