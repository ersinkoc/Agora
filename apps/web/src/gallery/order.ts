// The gallery's order and search (ADR-0120), plain functions over what both catalogs list. Popularity is the number the catalog
// publishes and `added` the day it first listed an entry; Agora counts nothing itself.

export type SortKey = 'popular' | 'new' | 'name' | 'author';
export const SORT_KEYS: readonly SortKey[] = ['popular', 'new', 'name', 'author'];

/** What an entry needs for ordering and search. `added` is `YYYY-MM-DD` or empty, `popularity` 0 when the catalog gives none. */
export interface Listed {
  readonly id: string;
  readonly name: string;
  readonly author: string;
  readonly added: string;
  readonly popularity: number;
  readonly description?: string;
}

/** The order a gallery opens with: popular when any entry has a popularity, else new when any has a day, else by name. */
export function defaultSort(items: readonly Listed[]): SortKey {
  if (items.some((i) => i.popularity > 0)) return 'popular';
  if (items.some((i) => i.added !== '')) return 'new';
  return 'name';
}

const byName = (a: Listed, b: Listed) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id);

/** The entries in `key`'s order: most popular first, newest first (an entry with no day last), by name, or by author. Ties go by name,
 *  so the order never depends on the order the hub sent. */
export function sorted<T extends Listed>(items: readonly T[], key: SortKey): T[] {
  const list = [...items];
  switch (key) {
    case 'popular':
      return list.sort((a, b) => b.popularity - a.popularity || byName(a, b));
    case 'new':
      // `YYYY-MM-DD` orders as text; an empty day sorts after every day.
      return list.sort((a, b) => (b.added || '').localeCompare(a.added || '') || byName(a, b));
    case 'author':
      return list.sort((a, b) => a.author.localeCompare(b.author) || byName(a, b));
    default:
      return list.sort(byName);
  }
}

/** Whether an entry matches what the person typed: every word appears in its name, author or description, ignoring case. */
export function matches(item: Listed, query: string): boolean {
  const words = query.toLocaleLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return true;
  const hay = `${item.name} ${item.author} ${item.description ?? ''}`.toLocaleLowerCase();
  return words.every((w) => hay.includes(w));
}

/** A screenshot as an address an `<img>` can show: only a PNG or WebP whose first bytes say what the hub said it is, as a `data:`
 *  address with that fixed type (the page policy allows `img-src data:`); `undefined` for anything else. */
export function screenshotAddress(mediaType: string, data: Uint8Array): string | undefined {
  const png = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
  const isPng = png.every((b, i) => data[i] === b);
  const ascii = (from: number, text: string) => [...text].every((c, i) => data[from + i] === c.charCodeAt(0));
  const isWebp = ascii(0, 'RIFF') && ascii(8, 'WEBP');
  if (!((mediaType === 'image/png' && isPng) || (mediaType === 'image/webp' && isWebp))) return undefined;
  let bin = '';
  for (let i = 0; i < data.length; i += 0x8000) bin += String.fromCharCode(...data.subarray(i, i + 0x8000));
  return `data:${mediaType};base64,${btoa(bin)}`;
}
