// Which maps the person has taken the walkthrough of, so the dock stops drawing the eye to it once they have. Testers
// missed the walkthrough and read the map alone, which is harder; once they took it, it helped. Kept in localStorage
// as a short list of map ids, newest last, capped so it can't grow forever.
export const TOURED_KEY = 'nonio-toured';
export const TOURED_CAP = 100;

/** The stored list, or an empty one if it's missing or not a list of ids (someone edited it, an older format). */
export function parseToured(raw: string | null): string[] {
  if (!raw) return [];
  try {
    const v: unknown = JSON.parse(raw);
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : [];
  } catch {
    return [];
  }
}

/** The list with this map added as the newest, dropping the oldest past the cap. */
export function withToured(list: readonly string[], mapId: string, cap = TOURED_CAP): string[] {
  return [...list.filter((x) => x !== mapId), mapId].slice(-cap);
}

