// Fuzzy matching for the command palette: every query character must appear in order. Matches that start a word,
// follow the previous match directly, or sit early score higher. Case-insensitive; empty query matches everything.

export function fuzzyScore(query: string, text: string): number | null {
  const q = query.trim().toLowerCase();
  if (q === '') return 0;
  const t = text.toLowerCase();
  let score = 0;
  let from = 0;
  let prev = -2;
  for (const ch of q) {
    const i = t.indexOf(ch, from);
    if (i < 0) return null;
    score += 1;
    if (i === prev + 1) score += 3; // consecutive
    if (i === 0 || /[\s\-_·/.]/.test(t[i - 1] ?? '')) score += 2; // word start
    score -= Math.min(i - from, 5) * 0.1; // gaps cost a little
    prev = i;
    from = i + 1;
  }
  return score - t.length * 0.01; // shorter text wins ties
}

export function rank<T>(items: readonly T[], query: string, textOf: (item: T) => string): T[] {
  return items
    .map((item, i) => ({ item, i, s: fuzzyScore(query, textOf(item)) }))
    .filter((x): x is { item: T; i: number; s: number } => x.s !== null)
    .sort((a, b) => b.s - a.s || a.i - b.i)
    .map((x) => x.item);
}
