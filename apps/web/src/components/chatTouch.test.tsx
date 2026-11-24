// @vitest-environment jsdom
// #1073: on a touch screen every control inside a chat entry takes a finger (a 44 px hit area, --tap), and the small ones keep their
// drawn size. jsdom cannot lay out or hit-test, so this checks the wiring: each control carries a class whose touch rule gives it --tap,
// and those rules are in the coarse-pointer block only (a mouse keeps the dense size). `scripts/e2e-chat-touch.mjs` measures the real
// hit areas in Chromium at 390 × 844.
import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';
import { beforeAll, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage } from '../i18n';
import { demoConversation, demoRichConversation } from '../demoChat';
import type { ChatLine } from '../net/chat';
import { transcript } from '../net/transcript';
import { FilesChanged, Item } from './ChatEntry';

const css = readFileSync('src/design/app.css', 'utf8').replace(/\r\n/g, '\n');
const coarse = /@media \(pointer: coarse\) \{([\s\S]*?)\n\}/.exec(css)?.[1] ?? '';

// What gives a control --tap on a touch screen, and how: a rule of the coarse block, or a `pointer-coarse:` utility of at least 44 px.
const TOUCH = [
  'tap-target', // an invisible box around it, --tap each way it falls short (chips, underlined text buttons)
  'tap-inline', // padding above and below each piece of a link in a sentence
  'ide-iconbtn', // --tap both ways (the Copy buttons)
  'button', // --tap tall
  'pointer-coarse:min-h-11', // rows and full-width buttons: 2.75rem = 44 px
  'pointer-coarse:py-3.5', // a line of small text (1rem) with 0.875rem above and below: 44 px
];

beforeAll(() => {
  // The Copy buttons are drawn only where the browser can copy.
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: async () => {} } });
});
beforeEach(() => setLanguage('en'));

const L = (id: number, over: Partial<ChatLine>): ChatLine => ({ id, atMs: 0, role: 'agent', kind: 'text', text: '', path: '', ...over });
// Beyond the gallery's two conversations: a call that printed more than its tail, a change of more than 200 lines, and two plans (the
// first is folded as an earlier plan).
const extra: ChatLine[] = [
  L(901, { kind: 'plan', text: 'one\ntwo' }),
  L(902, { kind: 'tool', text: '{"tool":"execute","arguments":{"command":"make"}}', callId: 'c902', callKind: 'run', status: 'completed', output: Array.from({ length: 40 }, (_, i) => `line ${i}`).join('\n') }),
  L(903, { kind: 'diff', path: 'big.rs', text: `--- a/big.rs\n+++ b/big.rs\n@@ -1,1 +1,300 @@\n${Array.from({ length: 300 }, (_, i) => `+l${i}`).join('\n')}` }),
  L(904, { kind: 'plan', text: 'one\ntwo\nthree' }),
];

function drawn(lines: ChatLine[]): HTMLElement {
  const host = document.createElement('div');
  host.innerHTML = renderToStaticMarkup(<>
    {transcript(lines).map((i) => <Item key={i.id} item={i} notice={() => null} onShowRequest={() => {}} />)}
    <FilesChanged lines={lines} onGo={() => {}} onReview={() => {}} />
  </>);
  return host;
}
const controls = (host: HTMLElement) => [...host.querySelectorAll<HTMLElement>('a[href], button, summary')];
const name = (el: HTMLElement) => `${el.tagName.toLowerCase()} "${(el.getAttribute('aria-label') || el.textContent || '').trim().slice(0, 40)}"`;

describe('#1073: a 44 px hit area for every control in a chat entry on a touch screen', () => {
  it('the coarse-pointer block has the invisible target and the inline padding, both sized from --tap, and nothing outside it does', () => {
    expect(coarse).toMatch(/\.tap-target \{ position: relative; \}/);
    expect(coarse).toMatch(/\.tap-target::before \{ content: ''; position: absolute; inset-block: min\(0px, calc\(\(100% - var\(--tap\)\) \/ 2\)\); inset-inline: min\(0px, calc\(\(100% - var\(--tap\)\) \/ 2\)\); \}/);
    expect(coarse).toMatch(/\.tap-inline \{ position: relative; padding-block: calc\(var\(--tap\) \/ 2 - 0\.5em\); \}/);
    // A link's words stay above a neighbouring link's reach.
    expect(coarse).toMatch(/\.tap-inline > span \{ position: relative; z-index: 1; \}/);
    const outside = css.replace(coarse, '');
    expect(outside).not.toMatch(/\.tap-(target|inline)/);
    // The rules the other classes rely on.
    expect(coarse).toMatch(/\.button,[^{]*\{ min-height: var\(--tap\); \}/);
    expect(coarse).toMatch(/\.ide-iconbtn, \.menu-tile \{ min-width: var\(--tap\); \}/);
  });

  for (const [label, lines] of [['the plain conversation', demoConversation()], ['the rich conversation', demoRichConversation()], ['long output, a long change, an earlier plan', extra]] as const) {
    it(`every link, button and fold in ${label} carries a touch rule`, () => {
      const list = controls(drawn([...lines]));
      expect(list.length).toBeGreaterThan(3);
      for (const el of list) expect(TOUCH.some((c) => el.classList.contains(c)), name(el)).toBe(true);
    });
  }

  it('the file chips and the "redirect notes" link keep their drawn size and grow an invisible target instead', () => {
    const host = drawn(demoConversation());
    const chips = [...host.querySelectorAll<HTMLElement>('[data-files-changed] li button')];
    expect(chips.length).toBeGreaterThanOrEqual(2);
    for (const chip of chips) {
      expect(chip.classList.contains('tap-target'), name(chip)).toBe(true);
      expect(chip.classList.contains('h-6'), name(chip)).toBe(true);
      expect([...chip.classList].some((c) => c.startsWith('pointer-coarse:')), name(chip)).toBe(false);
    }
    // Their rows sit far enough apart that two targets never overlap: 1.5rem chips, 1.25rem between rows on touch, and 0.75rem to the
    // Review button when it wraps under them (a chip's target reaches 10 px past it).
    expect(host.querySelector('[data-files-changed] ul')?.classList.contains('pointer-coarse:gap-y-5')).toBe(true);
    expect(host.querySelector('[data-files-changed]')?.classList.contains('pointer-coarse:gap-y-3')).toBe(true);
    const link = [...host.querySelectorAll<HTMLElement>('a[href]')].find((a) => a.textContent === 'redirect notes');
    expect(link?.classList.contains('tap-inline')).toBe(true);
    // Its words are in the span the rule raises, and the entry keeps that order to itself.
    expect(link?.firstElementChild?.tagName).toBe('SPAN');
    expect(link?.closest('[data-role=agent]')?.classList.contains('isolate')).toBe(true);
  });

  it('a reply\'s Copy sits under the reply on touch, never over a code block\'s own Copy', () => {
    const host = drawn(demoConversation());
    const copy = host.querySelector<HTMLElement>('[data-role=agent] > button');
    expect(copy?.getAttribute('aria-label')).toBe('Copy reply');
    expect(copy?.classList.contains('absolute')).toBe(true); // with a mouse: over the top corner, shown on hover
    expect(copy?.classList.contains('pointer-coarse:static')).toBe(true);
  });
});
