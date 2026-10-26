// Licensed Bible text (YouVersion's NIV, NASB, ... see src/storage/db/licensed.rs) may be shown but not
// kept. The server never stores or exports it; anything the console keeps for itself goes through here too.

/** One-line stand-in for a licensed slide's text, the same wording the server uses. */
function placeholder(item: any, slide: any): string {
  const what = slide?.reference_label || slide?.header || item?.title || 'Passage';
  return `${what} — fetching the text…`;
}

/** A copy of `schedule` that is safe to store: licensed passages keep their reference and lose their text. */
export function stripLicensedText<T extends { items?: any[] }>(schedule: T): T {
  if (!schedule || !Array.isArray(schedule.items) || !schedule.items.some((i) => i && i.licensed)) return schedule;
  return {
    ...schedule,
    items: schedule.items.map((item) => {
      if (!item || !item.licensed) return item;
      return {
        ...item,
        licensed: { ...item.licensed, resolved: false },
        slides: (item.slides || []).map((s: any) => ({ ...s, text: placeholder(item, s), elements: [], speaker_notes: null })),
      };
    }),
  };
}
