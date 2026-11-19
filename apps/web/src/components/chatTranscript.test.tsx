import { beforeEach, describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { setLanguage } from '../i18n';
import { demoConversation } from '../demoChat';
import type { ChatLine } from '../net/chat';
import { transcript } from '../net/transcript';
import { FilesChanged, Item } from './ChatEntry';

const line = (id: number, text: string, over: Partial<ChatLine> = {}): ChatLine => ({ id, atMs: 0, role: 'agent', kind: 'text', text, path: '', ...over });
const draw = (lines: ChatLine[]) => renderToStaticMarkup(<>{transcript(lines).map((i) => <Item key={i.id} item={i} notice={() => null} />)}</>);

describe('the conversation as drawn', () => {
  beforeEach(() => setLanguage('en'));
  it('draws a long realistic history: separators, folded tool groups with a one-line summary, diff cards, one latest plan', () => {
    const items = transcript(demoConversation());
    expect(items.filter((i) => i.kind === 'date')).toHaveLength(2);
    expect(items.filter((i) => i.kind === 'plan' && i.latest)).toHaveLength(1);
    const html = draw(demoConversation());
    // Tool groups are <details> without `open`; each says what it did in one line.
    expect(html).toMatch(/<details class="group max-w-3xl[^>]*data-steps="5"/);
    expect(html).not.toMatch(/data-steps="\d+"[^>]*\sopen/);
    expect(html).toContain('Read 3 files');
    expect(html).toContain('Ran a command');
    // Diffs are cards with a file name and +/- counts.
    expect(html).toContain('data-diff-path="src/retry.rs"');
    expect(html).toContain('login_ci.rs');
    // The user's message and the agent's reply are told apart, by role and by text for a screen reader.
    expect(html).toContain('data-role="user"');
    expect(html).toContain('data-role="agent"');
  });
  it('never draws what an agent wrote as HTML, and only web links as links', () => {
    const html = draw([line(1, '<img src=x onerror=alert(1)> [bad](javascript:alert(1)) [ok](https://example.com/a) <script>x</script>')]);
    expect(html).not.toContain('<img');
    expect(html).not.toContain('<script');
    expect(html).toContain('&lt;img src=x onerror=alert(1)&gt;');
    expect(html).not.toMatch(/href="javascript:/i);
    expect(html).toContain('href="https://example.com/a"');
    expect(html).toContain('rel="noopener noreferrer"');
  });
  it('draws a table as a table and a code block with its language label and a copy place', () => {
    const html = draw([line(1, '| a | b |\n|---|---|\n| 1 | 2 |\n\n```rust\nfn main() {}\n```')]);
    expect(html).toContain('<table');
    expect(html).toContain('<th');
    expect(html).toContain('>rust<');
  });
  it('lists each changed file once with its totals', () => {
    const html = renderToStaticMarkup(<FilesChanged lines={transcript(demoConversation()).flatMap((i) => (i.kind === 'diff' ? [i.line] : []))} />);
    expect(html).toContain('5 files changed');
    expect(html).toContain('src/retry.rs');
  });
});
