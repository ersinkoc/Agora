import { readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, expect, it } from 'vitest';

// Amber means only "needs you", plus the quota warning sign (ADR-0102; visual-language.md §2). Errors, deletions, risky choices and
// destructive confirmations are the danger red (`text-danger`, `border-danger`, `bg-danger`), change counts and other facts are neutral
// tags, an offline machine is the idle grey. This test finds every use of the amber token or its classes in the web source and allows it
// only in the files below, each with how many uses it holds and why. A new amber use anywhere fails here: if it really is "needs you" (or
// the quota sign), add it to the list with its reason; otherwise pick the role that says what it means.
const AMBER =
  /\b(?:text|bg|border|ring|fill|stroke|outline|from|to|via|shadow|divide|decoration|caret|accent)-needs-you\b|--(?:state|color)-needs-you\b|\b(?:tint|line)-needs\b|\bkit-tone-needs\b|\bphone-card--needs\b|\btone(?:=["']|=\{[^}\n]*['"]|:\s*['"])needs['"]/g;

const ALLOWED: Record<string, { uses: number; why: string }> = {
  'design/tokens.css': { uses: 3, why: 'the token itself, Dark and Light' },
  'design/app.css': { uses: 23, why: 'the utility and its Tailwind colour, its tint and hairline, the kit tone, the state mark, the needs-you tile, the rail alert badge, the avatar dot, the terminal bell, and the muted syntax colours mixed from the theme (content, like the diff)' },
  'components/kit/KitGallery.tsx': { uses: 4, why: 'the gallery shows the Needs you stat tile and request callout' },
  'components/FleetConsole.tsx': { uses: 3, why: "Home's request cards, the Needs you stat tile and the Needs you count" },
  'components/AttentionLayer.tsx': { uses: 2, why: 'the attention toast and dialog title' },
  'components/ApprovalBanner.tsx': { uses: 3, why: 'an approval request waiting on the person' },
  'components/SkipApproval.tsx': { uses: 2, why: 'a request to stop asking, waiting on the person' },
  'components/StartApproval.tsx': { uses: 1, why: 'a request to start a helper, waiting on the person' },
  'components/ChatEntry.tsx': { uses: 5, why: 'a tool group whose agent waits on the person, and a pending permission in the chat' },
  'components/SessionBar.tsx': { uses: 3, why: 'the Needs you session state, and the note to answer the request first' },
  'components/Console.tsx': { uses: 1, why: 'the status bar count of agents that need you' },
  'components/Inspector.tsx': { uses: 1, why: 'the Needs you list in the side panel' },
  'layout/TabStrip.tsx': { uses: 1, why: 'the dot on a tab whose agent needs you' },
  'layout/Workspace.tsx': { uses: 1, why: "the top ring of a pane whose agent needs you" },
  'phone/Phone.tsx': { uses: 8, why: 'the phone lists of what needs you, pairing requests and the next agent that needs you' },
  'phone/phone.css': { uses: 3, why: 'the phone request card (its class and colour) and the bar of what needs you' },
  'components/Quota.tsx': { uses: 11, why: 'the quota warning sign (triangle and percentage)' },
  'components/UsagePanel.tsx': { uses: 1, why: 'a usage meter at 80 % or more: the quota warning sign family (the audit lists it under quota)' },
  'components/sessionChoice.ts': { uses: 2, why: 'the context meter at 80 % or more, the quota warning sign family' },
  'terminal/renderer.ts': { uses: 1, why: 'terminal search matches: highlighted content, not chrome (the current match is the accent)' },
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
    // Not amber: the state's locale key, the rail tab id and the danger role.
    expect(hits("t('state-needs-you') { id: 'needs' } text-danger state--needs")).toBe(0);
  });

  it('is used only where something needs you or for the quota warning, and every listed use is still there', () => {
    const found = uses();
    const unlisted = [...found].filter(([f]) => !ALLOWED[f]).map(([f, n]) => `${f}: ${n}`);
    expect.soft(unlisted, 'amber outside the allowlist: use the danger role, a neutral tag or the idle grey').toEqual([]);
    const moved = Object.entries(ALLOWED).filter(([f, a]) => (found.get(f) ?? 0) !== a.uses).map(([f, a]) => `${f}: ${found.get(f) ?? 0} (listed ${a.uses})`);
    expect.soft(moved, 'an allowlisted file gained or lost an amber use: check it means "needs you", then update its count').toEqual([]);
  });
});
