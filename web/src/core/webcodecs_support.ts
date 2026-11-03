// Kept apart from webcodecs_player.ts so live_output.ts can ask this without
// pulling in mediabunny (~425KB minified): the player module is only fetched,
// via dynamic import, once a video background actually takes the WebCodecs path.
export function isWebCodecsSupported(): boolean {
  return typeof window.VideoDecoder !== 'undefined' && typeof window.AudioDecoder !== 'undefined';
}
