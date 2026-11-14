import { describe, expect, it } from 'vitest';
import { setLanguage, t } from '../i18n';
import { detailText, titleSays, toolView, type ToolAction } from './tool';

// The node's tool-call text: `{"tool": <title>, "arguments": <raw input>}` (crates/agora-node/src/chat.rs `describe`).
const call = (tool: string, args: string) => `{"tool":${JSON.stringify(tool)},"arguments":${args}}`;

describe('tool calls in words', () => {
  it('keeps the agent\'s title and picks the command as the subject', () => {
    const v = toolView(call('Run tests', '{"command":"cargo test -p agora-core","timeout":120}'));
    expect(v).toMatchObject({ action: 'run', title: 'Run tests', subject: 'cargo test -p agora-core', raw: '' });
    expect(v.details).toEqual([['command', 'cargo test -p agora-core'], ['timeout', '120']]);
  });
  it('says the action itself when the title is only an ACP kind or the fallback word', () => {
    expect(toolView(call('read', '{"path":"src/a.rs"}'))).toMatchObject({ action: 'read', title: '', subject: 'src/a.rs' });
    expect(toolView(call('execute', '{"command":["bash","-lc","ls -la"]}'))).toMatchObject({ action: 'run', title: '', subject: 'ls -la' });
    expect(toolView(call('tool', 'null'))).toMatchObject({ action: 'other', title: '', subject: '', details: [] });
  });
  it('tells an edit from a read by its arguments', () => {
    expect(toolView(call('Edit `src/auth.rs`', '{"file_path":"src/auth.rs","old_string":"a","new_string":"b"}')).action).toBe('edit');
    expect(toolView(call('Read src/auth.rs', '{"file_path":"src/auth.rs"}')).action).toBe('read');
    expect(toolView(call('grep', '{"pattern":"TODO","path":"src"}'))).toMatchObject({ action: 'search', subject: 'TODO' });
    expect(toolView(call('WebFetch', '{"url":"https://example.com"}'))).toMatchObject({ action: 'fetch', subject: 'https://example.com' });
  });
  it('shows arguments that are not an object as they are', () => {
    expect(toolView(call('echo', '"hi"'))).toMatchObject({ subject: 'hi', details: [['', 'hi']] });
    expect(toolView(call('many', '[1,2]')).details).toEqual([['', '[\n  1,\n  2\n]']]);
  });
  it('keeps every argument whole for the folded part', () => {
    const long = 'z'.repeat(5000);
    expect(toolView(call('Write', `{"file_path":"a.txt","content":"${long}"}`)).details).toEqual([['file_path', 'a.txt'], ['content', long]]);
  });
  it('reads a call the node cut short', () => {
    const cut = call('Write file', `{"file_path":"big.txt","content":"${'x'.repeat(200)}`).slice(0, 120);
    const v = toolView(cut);
    expect(v).toMatchObject({ action: 'edit', title: 'Write file', subject: 'big.txt', raw: '' });
    expect(v.details[0]?.[1]).toContain('"file_path"');
  });
  it('reads a call whose arguments were never JSON', () => {
    expect(toolView('{"tool":"shell","arguments":run it now}')).toMatchObject({ title: 'shell', details: [['', 'run it now']] });
  });
  it('falls back to the text as it came when it is not a tool call', () => {
    expect(toolView('ls -la')).toEqual({ action: 'other', title: '', subject: '', details: [], raw: 'ls -la' });
    expect(toolView('')).toMatchObject({ raw: '' , title: '' });
  });
  it('does not say the subject twice', () => {
    expect(titleSays('Read src/auth.rs', 'src/auth.rs')).toBe(true);
    expect(titleSays('Read auth.rs', '/home/dev/src/auth.rs')).toBe(true);
    expect(titleSays('Run tests', 'cargo test')).toBe(false);
    expect(titleSays('', 'x')).toBe(false);
  });
  it('has words for every action in every language', () => {
    const actions: ToolAction[] = ['read', 'edit', 'run', 'search', 'fetch', 'delete', 'move', 'think', 'other'];
    for (const lang of ['en', 'tr']) {
      setLanguage(lang);
      for (const a of actions) expect(t(`tool-${a}`), `${lang} ${a}`).not.toBe(`tool-${a}`);
    }
    setLanguage('en');
  });
});

describe('approval details in words', () => {
  it('names the command or file of a chat agent\'s request', () => {
    expect(detailText('{"command":"rm -rf target","description":"clean"}')).toBe('rm -rf target');
    expect(detailText('{"file_path":"src/a.rs","content":"fn main() {}"}')).toBe('src/a.rs');
  });
  it('reads a detail cut at 300 characters', () => {
    expect(detailText(`{"content":"${'y'.repeat(250)}","file_path":"late.rs"`.slice(0, 120) + '…')).toBe(`{"content":"${'y'.repeat(250)}","file_path":"late.rs"`.slice(0, 120) + '…');
    expect(detailText('{"file_path":"src/a.rs","content":"aaaa…')).toBe('src/a.rs');
  });
  it('lists plain fields when no subject is there, and leaves anything else alone', () => {
    expect(detailText('{"mode":"fast","n":2,"x":{"y":1}}')).toBe('mode: fast · n: 2');
    expect(detailText('+12 −3')).toBe('+12 −3');
    expect(detailText('{not json')).toBe('{not json');
    expect(detailText('{"x":{"y":1}}')).toBe('{"x":{"y":1}}');
  });
});
