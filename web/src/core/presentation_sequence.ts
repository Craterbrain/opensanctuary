/**
 * OpenSanctuary / OS-Next Universal Presentation Sequencing & Next-Item Resolver
 *
 * Unifies next-slide lookahead, next-service-item rollover, bilingual slide
 * splitting, and end-of-schedule determination across stage/confidence monitors,
 * prompter displays, and mobile remote control views.
 */

import { resolveSlideAt } from './presentation_helpers.ts';

export interface BilingualText {
  isBilingual: boolean;
  primary: string;
  secondary: string;
}

export interface NextPresentationPreview {
  /** True if this preview is from the same item's next slide */
  isNextSlide: boolean;
  /** True if this preview is the first slide of the upcoming schedule item */
  isNextScheduleItem: boolean;
  /** True if at the very end of the schedule with no more items or slides */
  isEndOfSchedule: boolean;
  /** Descriptive label e.g. "Verse 2", "Slide 3", "NEXT SERVICE ITEM", or "" */
  label: string;
  /** Full text or formatted preview text */
  text: string;
  /** Split bilingual components if '|||' was present */
  bilingual: BilingualText;
  /** Reference to the resolved slide object, if any */
  slide: any | null;
  /** Reference to the next schedule item object, if rolling over to next item */
  nextItem: any | null;
}

/**
 * Splits text containing '|||' into bilingual components.
 */
export function parseBilingualSlideText(text: string | null | undefined): BilingualText {
  if (!text) {
    return { isBilingual: false, primary: '', secondary: '' };
  }
  if (text.includes('|||')) {
    const [l, r] = text.split('|||');
    return {
      isBilingual: true,
      primary: (l || '').trim(),
      secondary: (r || '').trim()
    };
  }
  return {
    isBilingual: false,
    primary: text.trim(),
    secondary: ''
  };
}

/**
 * Resolves the next presentation preview (next slide, next schedule item, or end of schedule).
 *
 * @param schedule The current schedule (e.g. `{ items: [...] }`)
 * @param liveItem The currently active live item (or null)
 * @param liveSlideIndex The current 0-indexed slide position in the live item
 */
export function resolveNextPresentationPreview(
  schedule: { items?: any[] } | null | undefined,
  liveItem: any | null | undefined,
  liveSlideIndex: number
): NextPresentationPreview {
  if (!liveItem) {
    return {
      isNextSlide: false,
      isNextScheduleItem: false,
      isEndOfSchedule: true,
      label: '',
      text: '',
      bilingual: { isBilingual: false, primary: '', secondary: '' },
      slide: null,
      nextItem: null
    };
  }

  // 1. Check for a next slide in the current live item
  const nextSlide = resolveSlideAt(liveItem, liveSlideIndex + 1);
  if (nextSlide) {
    const label = nextSlide.label || `Slide ${liveSlideIndex + 2}`;
    const rawText = nextSlide.text || `[Next: ${nextSlide.label || 'Slide'}]`;
    const bilingual = parseBilingualSlideText(nextSlide.text);

    return {
      isNextSlide: true,
      isNextScheduleItem: false,
      isEndOfSchedule: false,
      label,
      text: rawText,
      bilingual,
      slide: nextSlide,
      nextItem: null
    };
  }

  // 2. Rollover to the next item in the schedule
  const items = schedule?.items || [];
  let nextScheduleItem: any = null;
  if (items.length > 0) {
    const curIdx = items.findIndex((it: any) => it.id === liveItem.id);
    if (curIdx >= 0 && curIdx + 1 < items.length) {
      nextScheduleItem = items[curIdx + 1];
    }
  }

  if (nextScheduleItem) {
    const firstSlide = resolveSlideAt(nextScheduleItem, 0) || (nextScheduleItem.slides && nextScheduleItem.slides[0]);
    const firstText = firstSlide ? (firstSlide.text || '') : '';
    const bilingual = parseBilingualSlideText(firstText);
    const snippet = firstText ? `${firstText.substring(0, 80)}...` : '';
    const text = `[ Next: ${nextScheduleItem.title || 'Untitled'} ]${snippet ? `\n${snippet}` : ''}`;

    return {
      isNextSlide: false,
      isNextScheduleItem: true,
      isEndOfSchedule: false,
      label: 'NEXT SERVICE ITEM',
      text,
      bilingual,
      slide: firstSlide || null,
      nextItem: nextScheduleItem
    };
  }

  // 3. Reached end of schedule
  return {
    isNextSlide: false,
    isNextScheduleItem: false,
    isEndOfSchedule: true,
    label: '',
    text: '[ End of Service Schedule ]',
    bilingual: { isBilingual: false, primary: '', secondary: '' },
    slide: null,
    nextItem: null
  };
}
