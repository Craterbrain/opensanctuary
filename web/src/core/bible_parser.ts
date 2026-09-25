/**
 * OpenSanctuary / OS-Next Bible Reference Parser
 * High-speed local regex tokenizer + async Rust Engine validation bridge.
 */

import { ScriptureReferenceResult } from "./presentation_helpers.ts";

export const parsedRefCache = new Map<string, ScriptureReferenceResult>();

export function setParsedRefCache(key: string, val: ScriptureReferenceResult) {
  if (parsedRefCache.size > 500) {
    const firstKey = parsedRefCache.keys().next().value;
    if (firstKey) parsedRefCache.delete(firstKey);
  }
  parsedRefCache.set(key, val);
}

export async function parseScriptureReferenceAsync(input: string): Promise<ScriptureReferenceResult> {
  if (!input || !input.trim()) return { isReference: false };
  const str = input.trim();
  if (parsedRefCache.has(str)) return parsedRefCache.get(str)!;

  try {
    const res = await fetch(`/api/bibles/parse-ref?q=${encodeURIComponent(str)}`);
    if (res.ok) {
      const data = await res.json();
      const result: ScriptureReferenceResult = data || { isReference: false };
      setParsedRefCache(str, result);
      return result;
    }
  } catch (e) {
    console.warn('[BibleParser] Error fetching parsed scripture ref:', e);
  }
  return { isReference: false };
}

/**
 * Fast synchronous lookup with background Rust validation.
 * @param input Raw search text (e.g. "John 3:16", "1 Cor 13")
 * @param onAsyncResolved Optional callback invoked when background Rust resolution returns.
 */
export function parseScriptureReference(
  input: string,
  onAsyncResolved?: (res: ScriptureReferenceResult) => void
): ScriptureReferenceResult {
  if (!input || !input.trim()) return { isReference: false };
  const str = input.trim();
  if (parsedRefCache.has(str)) return parsedRefCache.get(str)!;

  // Trigger background resolution from Rust engine
  parseScriptureReferenceAsync(str).then((res) => {
    if (res && res.isReference && onAsyncResolved) {
      onAsyncResolved(res);
    }
  });

  // Fast synchronous regex tokenization
  const m = str.match(/^([1-3]?\s*[a-zA-Z]+(?:\s+of\s+[a-zA-Z]+)?)\s*(\d+)?(?::(\d+)(?:-(\d+))?)?/);
  if (m && m[1]) {
    const bookGuess = m[1].trim();
    const ch = m[2] ? parseInt(m[2], 10) : null;
    const vs = m[3] ? parseInt(m[3], 10) : null;
    const ve = m[4] ? parseInt(m[4], 10) : vs;
    return {
      isReference: true,
      book: bookGuess,
      chapter: ch,
      verseStart: vs,
      verseEnd: ve,
      hasSpecificVerse: !!vs,
      formatted: str
    };
  }

  return { isReference: false };
}
