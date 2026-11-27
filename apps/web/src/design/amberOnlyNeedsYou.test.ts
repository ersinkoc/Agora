import { readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, expect, it } from 'vitest';

// Amber means only "needs you", plus the quota warning sign (ADR-0102; visual-language.md §2). Errors, deletions, risky choices and
// destructive confirmations are the danger red (`text-danger`, `border-danger`, `bg-danger`), change counts and other facts are neutral
// tags, an offline machine is the idle grey. This test finds every use of the amber token or its classes in the web source and allows it
// only in the files below, each with how many uses it holds and why. A new amber use anywhere fails here: if it really is "needs you" (or
// the quota sign), add it to the list with its reason; otherwise pick the role that says what it means.
// The kit's amber tone is the word `needs` (`<Pill tone="needs">`, `kit-tone-${…}`), and a tone can come out of a table or a branch
// (`{ 'needs-you': 'needs' }`, `x ? 'needs' : …`) as well as a prop, so every quoted needs (any quote) counts (#1082): no table can hide one. The
// rail's Needs you tab has the same id, `'needs'`; the files that name that tab are listed apart, in TAB_ID, with their own counts.
const AMBER =
  /\b(?:text|bg|border|ring|fill|stroke|outline|from|to|via|shadow|divide|decoration|caret|accent)-needs-you\b|--(?:state|color)-needs-you\b|\b(?:tint|line)-needs\b|\bkit-tone-needs\b|\bphone-card--needs\b|\btone(?:=["']|=\{[^}\n]*['"]|:\s*['"])needs['"]|(['"`])needs\1/g;

const ALLOWED: Record<string, { uses: number; why: string }> = {
  'design/tokens.css': { uses: 3, why: 'the token itself, Dark and Light' },
  'design/app.css': { uses: 32, why: 'the utility and its Tailwind colour, its tint and hairline, the kit tone, the state mark, the needs-you tile, the rail alert badge, the avatar dot, the map\'s ring of an agent that needs you (ADR-0129), a disk running low (ADR-0136), the terminal bell, the muted syntax colours mixed from the theme (content, like the diff), and the theme floor that keeps "needs you" out of a theme\'s state display, regions and fonts (ADR-0113)' },
  'components/kit/KitGallery.tsx': { uses: 4, why: 'the gallery shows the Needs you stat tile and request callout' },
  'components/FleetConsole.tsx': { uses: 2, why: 'the Needs you stat tile and the Needs you count (its request cards are ApprovalRequest)' },
  'components/AttentionLayer.tsx': { uses: 1, why: 'the attention toast (the dialog is ApprovalRequest)' },
  'components/ApprovalRequest.tsx': { uses: 1, why: 'the one approval request component (redesign PR 4): every request waiting on the person, starts and switches included (PR 5)' },
  'components/kit/Pill.tsx': { uses: 2, why: "the kit's Tone type and its one state-to-tone table (Needs you is amber), which every status pill and mark uses" },
  'components/kit/Callout.tsx': { uses: 3, why: 'the callout tone of a request that waits on the person, its kit tone, and the comment that says so' },
  'themes/tints.ts': { uses: 1, why: 'the needs tint among the tint roles the theme mixes' },
  'components/ChatEntry.tsx': { uses: 5, why: 'a tool group whose agent waits on the person, and a pending permission in the chat' },
  'components/SessionBar.tsx': { uses: 1, why: "the note to answer the request first (the Needs you pill takes the kit's state tone, redesign PR 12)" },
  'components/Console.tsx': { uses: 1, why: 'the status bar count of agents that need you' },
  'components/Inspector.tsx': { uses: 1, why: 'the Needs you list in the side panel' },
  'layout/TabStrip.tsx': { uses: 1, why: 'the dot on a tab whose agent needs you' },
  'layout/Workspace.tsx': { uses: 1, why: "the top ring of a pane whose agent needs you" },
  'phone/Phone.tsx': { uses: 3, why: 'the Needs you count on the phone list and on its bar (its request cards are ApprovalRequest), and the needs tone of a request folded to one line' },
  'phone/phone.css': { uses: 6, why: 'the needs tint and hairline of an agent that waits, the hairline of the start and switch frames, and a request folded to one line on an agent (its tint, hairline and icon)' },
  'screens.tsx': { uses: 1, why: 'Settings: another device of yours waiting for your approval (redesign PR 8)' },
  'components/NetworkView.tsx': { uses: 1, why: 'My machines: the Needs you count in the hub strip, when an agent waits (redesign PR 9)' },
  'components/MachineGauges.tsx': { uses: 3, why: 'a disk running low needs you (ADR-0136): its words on the machine card, its line and the callout tone in Home\'s Needs you' },
  'components/PairRequest.tsx': { uses: 1, why: 'a request to join that waits on the owner: the needs tone of its card (redesign PR 9)' },
  'components/Quota.tsx': { uses: 11, why: 'the quota warning sign (triangle and percentage)' },
  'components/UsagePanel.tsx': { uses: 1, why: 'a usage meter at 80 % or more: the quota warning sign family (the audit lists it under quota)' },
  'components/sessionChoice.ts': { uses: 2, why: 'the context meter at 80 % or more, the quota warning sign family' },
};

// Not amber: the rail's Needs you tab, whose id is the word `'needs'` too. Each file's count is exact, so a tone added beside a tab id fails
// here as any other would.
const TAB_ID: Record<string, { uses: number; why: string }> = {
  'components/rail.ts': { uses: 4, why: "the tab id: in the RailTab type, the tab's entry, the fallback tab and the tab order" },
  'components/Inspector.tsx': { uses: 3, why: 'the tab id: opening the Needs you tab and showing its list' },
  'components/Console.tsx': { uses: 3, why: 'the tab id: the first tab, and Review and the status bar count opening it' },
  'layout/builtinPanels.tsx': { uses: 1, why: 'the tab id: the Needs you panel (approvals) draws that tab of the side panel' },
};

const SRC = 'src';
const walk = (dir: string): string[] =>
  readdirSync(dir, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(join(dir, e.name)) : [join(dir, e.name)]));
// Paths use `/` on every OS, and a Windows checkout's CRLF line ends do not change what matches.
const uses = (): Map<string, number> => {
  const found = new Map<string, number>();
  for (const path of walk(SRC)) {
    if (!/\.(tsx?|css)$/.test(path) || /\.test\.tsx?$/.test(path)) continue;
    const n = (readFileSync(path, 'utf8').replace(/\r\n/g, '\n').match(AMBER) ?? []).length;
    if (n > 0) found.set(relative(SRC, path).split(/[\\/]/).join('/'), n);
  }
  return found;
};

describe('amber means only "needs you" (ADR-0102)', () => {
  it('finds the amber token and its classes in every form a screen writes them', () => {
    const hits = (s: string) => s.match(AMBER)?.length ?? 0;
    expect(hits('className="m-0 text-needs-you"')).toBe(1);
    expect(hits("hover:bg-needs-you/10 border-needs-you/60 ring-1 ring-needs-you")).toBe(3);
    expect(hits('border: 1px solid var(--state-needs-you);')).toBe(1);
    expect(hits('bg-tint-needs kit-tone-needs phone-card--needs')).toBe(3);
    expect(hits('<Pill tone="needs">')).toBe(1);
    expect(hits("tone: 'needs'")).toBe(1);
    expect(hits("tone={f === 'needs-you' ? 'needs' : undefined} tone={'needs'}")).toBe(2);
    expect(hits('color: var(--color-needs-you); text-(--color-needs-you)')).toBe(2);
    // A tone out of a table, a branch or a list (#1082: SessionBar's Needs you pill was amber through a table this test could not see).
    expect(hits("const PILL: Record<UiState, Tone> = { working: 'working', 'needs-you': 'needs', idle: 'idle' };")).toBe(1);
    expect(hits("const tone = failed ? 'danger' : waiting ? \"needs\" : 'plain';")).toBe(1);
    expect(hits("const TONE = { needs: 'needs', info: 'accent' }; const ROLES = ['accent', 'needs'] as const;")).toBe(2);
    // The rail's tab id is the same word: it counts, and the files that name it are listed in TAB_ID.
    expect(hits("{ id: 'needs' }")).toBe(1);
    // Not amber: the state and its locale key, a CSS state modifier, the danger role, a word that only starts with needs.
    expect(hits('const tone = x ? `needs` : `plain`;')).toBe(1);
    expect(hits("state=\"needs-you\" t('state-needs-you') 'needs-you' text-danger state--needs 'needsReview'")).toBe(0);
  });

  it('is used only where something needs you or for the quota warning, and every listed use is still there', () => {
    // A file that names the rail's tab holds its listed tab ids first; whatever is left is amber.
    const found = new Map([...uses()].map(([f, n]) => [f, n - (TAB_ID[f]?.uses ?? 0)] as const));
    const tabs = Object.entries(TAB_ID).filter(([f]) => (found.get(f) ?? -Infinity) < 0 || !found.has(f)).map(([f, a]) => `${f}: listed ${a.uses}`);
    expect.soft(tabs, 'a file listed in TAB_ID names the tab fewer times: update its count').toEqual([]);
    for (const [f, n] of found) if (n <= 0) found.delete(f);
    const unlisted = [...found].filter(([f]) => !ALLOWED[f]).map(([f, n]) => `${f}: ${n}`);
    expect.soft(unlisted, 'amber outside the allowlist: use the danger role, a neutral tag or the idle grey').toEqual([]);
    const moved = Object.entries(ALLOWED).filter(([f, a]) => (found.get(f) ?? 0) !== a.uses).map(([f, a]) => `${f}: ${found.get(f) ?? 0} (listed ${a.uses})`);
    expect.soft(moved, 'an allowlisted file gained or lost an amber use: check it means "needs you", then update its count').toEqual([]);
  });
});
