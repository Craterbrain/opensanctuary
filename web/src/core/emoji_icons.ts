/**
 * Replaces emoji in the console UI with the themed SVG icons (web/icons/, see
 * icons/emoji-map.json for which emoji maps to which icon).
 *
 * Instead of rewriting hundreds of strings, `startEmojiIcons()` converts mapped emoji in
 * rendered text into `<img data-icon>` elements -- once for the page and then for
 * everything added or changed later (toasts, dialogs, lists built from template strings).
 * Because the images carry `data-icon`, `applyIconSet` re-colors them when the theme
 * changes, exactly like the ribbon icons.
 *
 * Two escape hatches:
 *  - `{icon:name}` in a string renders that icon explicitly -- for the few emoji whose
 *    meaning differs by call site (e.g. 📖 as "User Guide" rather than "Bible").
 *  - `data-no-iconify` on an element (or `contenteditable`, inputs, the slide canvases)
 *    leaves its text alone: user content is never rewritten.
 */
import emojiMap from '../../icons/emoji-map.json';
import { getCurrentIconSet, iconUrl } from './icon_sets.ts';

interface EmojiMapEntry { glyph: string; icon: string | null; }

const VS16 = '️';
const TOKEN = '\\{icon:([a-z0-9-]+)\\}';

const glyphToIcon = new Map<string, string>();
for (const e of emojiMap as EmojiMapEntry[]) {
  if (e.icon) glyphToIcon.set(e.glyph.replaceAll(VS16, ''), e.icon);
}

const escapeRe = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
const glyphAlternation = [...glyphToIcon.keys()]
  .sort((a, b) => b.length - a.length)
  .map(escapeRe)
  .join('|');
const MATCHER = new RegExp(`(?:${glyphAlternation})${VS16}?|${TOKEN}`, 'gu');

export type TextPiece = string | { icon: string };

/** Splits text into plain runs and icon references. Pure, so it is unit-tested without a DOM. */
export function splitEmojiText(text: string): TextPiece[] {
  const pieces: TextPiece[] = [];
  let last = 0;
  for (const m of text.matchAll(MATCHER)) {
    const icon = m[1] ?? glyphToIcon.get(m[0].replaceAll(VS16, ''));
    if (!icon) continue;
    if (m.index! > last) pieces.push(text.slice(last, m.index));
    pieces.push({ icon });
    last = m.index! + m[0].length;
  }
  if (last < text.length) pieces.push(text.slice(last));
  return pieces;
}

/** The HTML for an icon, for strings assigned to innerHTML. */
export function iconHtml(name: string): string {
  return `<img class="ui-icon" data-icon="${name}" src="${iconUrl(getCurrentIconSet(), name)}" alt="" draggable="false">`;
}

function iconElement(name: string): HTMLImageElement {
  const img = document.createElement('img');
  img.className = 'ui-icon';
  img.dataset.icon = name;
  img.src = iconUrl(getCurrentIconSet(), name);
  img.alt = '';
  img.draggable = false;
  return img;
}

const SKIP_TAGS = new Set(['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEXTAREA', 'INPUT', 'SELECT', 'OPTION', 'TITLE']);
// Tree expand/collapse carets (▶ ▼) stay glyphs; so does anything inside the slide canvases.
const SKIP_SELECTOR = '[contenteditable], [data-no-iconify], .canvas-16-9, .group-caret, .schedule-caret';

function skip(el: Element | null): boolean {
  return !el || SKIP_TAGS.has(el.tagName) || el.closest(SKIP_SELECTOR) !== null;
}

function iconifyTextNode(node: Text): void {
  const text = node.nodeValue;
  if (!text || skip(node.parentElement)) return;
  const pieces = splitEmojiText(text);
  if (pieces.length === 1 && typeof pieces[0] === 'string') return;
  const frag = document.createDocumentFragment();
  for (const p of pieces) frag.append(typeof p === 'string' ? document.createTextNode(p) : iconElement(p.icon));
  node.replaceWith(frag);
}

/** Converts mapped emoji (and `{icon:name}` tokens) in every text node at or under `root`. */
export function iconifyNode(root: Node): void {
  if (root.nodeType === Node.TEXT_NODE) { iconifyTextNode(root as Text); return; }
  if (root.nodeType !== Node.ELEMENT_NODE || skip(root as Element)) return;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const found: Text[] = [];
  for (let n = walker.nextNode(); n; n = walker.nextNode()) found.push(n as Text);
  found.forEach(iconifyTextNode);
}

let observer: MutationObserver | null = null;

/**
 * Converts the page now and keeps converting what gets rendered later. Runs inside the
 * MutationObserver microtask, i.e. before the browser paints, so no emoji flashes.
 */
export function startEmojiIcons(root: HTMLElement = document.body): void {
  if (observer) return;
  iconifyNode(root);
  observer = new MutationObserver(records => {
    for (const r of records) {
      if (r.type === 'characterData') iconifyNode(r.target);
      else r.addedNodes.forEach(n => iconifyNode(n));
    }
    observer!.takeRecords(); // drop the mutations our own replacements just made
  });
  observer.observe(root, { childList: true, subtree: true, characterData: true });
}
