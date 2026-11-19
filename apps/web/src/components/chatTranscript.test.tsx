import { beforeEach, describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { setLanguage } from '../i18n';
import { demoConversation } from '../demoChat';
import type { ChatLine } from '../net/chat';
import { transcript } from '../net/transcript';
import { FilesChanged, Item, PlanCard, Thought, ToolSteps, Turn } from './ChatEntry';

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
  it('says "N steps" with a duration only when the entries carry times', () => {
    const call = (id: number, atMs: number) => line(id, '{"tool":"Read","arguments":{"file_path":"a.rs"}}', { kind: 'tool', atMs });
    const timed = renderToStaticMarkup(<ToolSteps lines={[call(1, 1_000_000), call(2, 1_004_100)]} />);
    expect(timed).toContain('2 steps · 4.1 s');
    const untimed = renderToStaticMarkup(<ToolSteps lines={[call(1, 0), call(2, 0), call(3, 0)]} />);
    expect(untimed).toContain('3 steps');
    expect(untimed).not.toMatch(/3 steps · /);
  });
  it('opens the newest group and the one that waits on you, and folds the rest', () => {
    const lines = [line(1, '{"tool":"Read","arguments":{}}', { kind: 'tool' })];
    expect(renderToStaticMarkup(<ToolSteps lines={lines} />)).not.toMatch(/<details[^>]*\sopen/);
    expect(renderToStaticMarkup(<ToolSteps lines={lines} open />)).toMatch(/<details[^>]*\sopen/);
    expect(renderToStaticMarkup(<ToolSteps lines={lines} flag="needs-you" />)).toMatch(/<details[^>]*\sopen/);
  });
  it('shows an edit with its diff inside the row, with line numbers and a hunk header', () => {
    const html = renderToStaticMarkup(<ToolSteps open lines={[line(1, '{"tool":"Edit","arguments":{"file_path":"src/a.rs","old_string":"a","new_string":"b"}}', { kind: 'tool' })]}
      diffs={new Map([[1, line(2, '@@ -39,2 +39,2 @@ fn f\n keep\n-old\n+new\n', { kind: 'diff', path: 'src/a.rs' })]])} />);
    expect(html).toContain('data-diff-path="src/a.rs"');
    expect(html).toContain('@@ -39,2 +39,2 @@ fn f');
    expect(html).toContain('>39<');
    expect(html).toContain('>40<');
  });
  it('draws a thinking block folded, with its time only when known, and the text as text', () => {
    const html = renderToStaticMarkup(<Thought text="<b>x</b>" seconds={9} />);
    expect(html).toContain('Thought for 9.0 s');
    expect(html).not.toMatch(/<details[^>]*\sopen/);
    expect(html).toContain('&lt;b&gt;x&lt;/b&gt;');
    expect(renderToStaticMarkup(<Thought text="t" />)).not.toContain('Thought for');
  });
  it('draws a plan as a numbered list, and ticks and a bar only when a status is given', () => {
    const plain = renderToStaticMarkup(<PlanCard steps={['a', 'b']} label="Plan" />);
    expect(plain).not.toContain('progressbar');
    expect(plain).not.toContain('line-through');
    expect(plain).toContain('>1<');
    const withStatus = renderToStaticMarkup(<PlanCard steps={['a', 'b', 'c', 'd']} status={['completed', 'completed', 'in_progress', 'pending']} label="Plan" />);
    expect(withStatus).toContain('aria-valuenow="50"');
    expect(withStatus).toContain('2 of 4 done');
    expect(withStatus).toContain('data-status="completed"');
  });
  it('puts an agent turn beside its avatar, name, model and time', () => {
    const html = renderToStaticMarkup(<Turn name="claude · amber" model="Opus" atMs={Date.now()}>x</Turn>);
    expect(html).toContain('>C<');
    expect(html).toContain('claude · amber');
    expect(html).toContain('Opus');
  });
  it('lists each changed file once with its totals', () => {
    const html = renderToStaticMarkup(<FilesChanged lines={transcript(demoConversation()).flatMap((i) => (i.kind === 'diff' ? [i.line] : []))} />);
    expect(html).toContain('5 files changed');
    expect(html).toContain('src/retry.rs');
  });
});
