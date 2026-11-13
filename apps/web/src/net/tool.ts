// A tool call in a conversation, in words. The node sends `{"tool": <title>, "arguments": <the agent's raw input>}` (ADR-0038);
// the title and the arguments were written by the agent, so they are shown as text. Pure: never throws, and anything it cannot
// read falls back to the text as it came.

// What kind of thing the agent did. `other` when nothing says.
export type ToolAction = 'read' | 'edit' | 'run' | 'search' | 'fetch' | 'delete' | 'move' | 'think' | 'other';

export interface ToolView {
  action: ToolAction;
  // The agent's own title ("Read src/auth.rs", "`cargo test`"); empty when the agent gave none worth showing.
  title: string;
  // The one argument a person wants to see first: the command, the file, the search, the address.
  subject: string;
  // Every argument as name and text, for a folded "details" part. Empty when there were none.
  details: [string, string][];
  // What came, unchanged, when it could not be read as a tool call at all.
  raw: string;
}

// Titles an agent sends when it has no better one: the ACP tool kinds, and the bridge's own fallback.
const KINDS: Record<string, ToolAction> = {
  read: 'read', edit: 'edit', delete: 'delete', move: 'move', search: 'search', execute: 'run', think: 'think', fetch: 'fetch',
  other: 'other', tool: 'other',
};
// The argument that says what a call is about, in the order they are looked for, and what each one suggests the call does.
// A search names the folder it looks in too, so what it looks for comes before any path.
const SUBJECTS: [string, ToolAction][] = [
  ['command', 'run'], ['cmd', 'run'], ['pattern', 'search'], ['query', 'search'], ['url', 'fetch'], ['uri', 'fetch'],
  ['file_path', 'read'], ['filePath', 'read'], ['notebook_path', 'read'], ['path', 'read'], ['abs_path', 'read'], ['description', 'other'],
];
const EDIT_KEYS = ['old_string', 'new_string', 'oldText', 'newText', 'content', 'edits', 'patch', 'diff', 'new_str', 'old_str'];
const isObject = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);

// A value as one would read it: text as it is, a command given as words joined, anything else as indented JSON.
function asText(v: unknown): string {
  if (typeof v === 'string') return v;
  if (typeof v === 'number' || typeof v === 'boolean') return String(v);
  if (Array.isArray(v) && v.every((x) => typeof x === 'string')) {
    // `["bash", "-lc", "cargo test"]`: the shell is noise, the script is the command.
    if (v.length === 3 && /(^|\/)(ba|z|da)?sh$|^(pwsh|powershell|cmd)(\.exe)?$/i.test(v[0] as string) && /^(-l?c|\/c|-Command)$/i.test(v[1] as string)) return v[2] as string;
    return v.join(' ');
  }
  try { return JSON.stringify(v, null, 2) ?? ''; } catch { return ''; }
}

// The first string a key holds in text that is not quite JSON (cut short, or an adapter's own format).
function stringField(text: string, key: string): string | undefined {
  const m = new RegExp(`"${key}"\\s*:\\s*("(?:[^"\\\\]|\\\\.)*")`).exec(text);
  if (!m) return undefined;
  try { return JSON.parse(m[1] as string) as string; } catch { return undefined; }
}

function pickSubject(args: Record<string, unknown>): { subject: string; hint: ToolAction } {
  for (const [key, hint] of SUBJECTS) {
    const v = args[key];
    if (v === undefined || v === null || v === '') continue;
    const s = asText(v);
    if (s && !s.includes('\n')) return { subject: s, hint };
    if (s && key !== 'description') return { subject: s.split('\n')[0] as string, hint };
  }
  return { subject: '', hint: 'other' };
}

function infer(keys: string[], hint: ToolAction): ToolAction {
  if (hint === 'run' || hint === 'search' || hint === 'fetch') return hint;
  if (keys.some((k) => EDIT_KEYS.includes(k))) return 'edit';
  return hint === 'read' ? 'read' : 'other';
}

// The bridge's fallback title is the ACP kind or `tool`: a word for the window to say, not a title.
function titled(tool: string): { title: string; kind?: ToolAction } {
  const kind = KINDS[tool.trim().toLowerCase()];
  return kind ? { title: '', kind } : { title: tool.trim() };
}

export function toolView(text: string): ToolView {
  let parsed: unknown;
  try { parsed = JSON.parse(text); } catch { parsed = undefined; }
  if (isObject(parsed) && typeof parsed.tool === 'string') {
    const { title, kind } = titled(parsed.tool);
    const args = parsed.arguments;
    if (isObject(args)) {
      const { subject, hint } = pickSubject(args);
      // Whole values: the row shows the short form, the folded part everything the agent sent (the node caps an entry at 16 KiB).
      const details = Object.entries(args).map(([k, v]): [string, string] => [k, asText(v)]);
      return { action: kind && kind !== 'other' ? kind : infer(Object.keys(args), hint), title, subject, details, raw: '' };
    }
    const shown = args === undefined || args === null ? '' : asText(args);
    // Folded too when it fits on one line: the row truncates a long one.
    return { action: kind ?? 'other', title, subject: shown.includes('\n') ? '' : shown, details: shown ? [['', shown]] : [], raw: '' };
  }
  // Cut short at the node's limit, or arguments that were never JSON: read what can be read, keep the rest as it came.
  const tool = stringField(text, 'tool');
  if (tool === undefined) return { action: 'other', title: '', subject: '', details: [], raw: text };
  const { title, kind } = titled(tool);
  let subject = '';
  let hint: ToolAction = 'other';
  for (const [key, h] of SUBJECTS) {
    const s = stringField(text, key);
    if (s) { subject = s.split('\n')[0] as string; hint = h; break; }
  }
  const at = text.indexOf('"arguments"');
  const rest = at >= 0 ? text.slice(text.indexOf(':', at) + 1).replace(/\}$/, '').trim() : '';
  return { action: kind && kind !== 'other' ? kind : infer(EDIT_KEYS.filter((k) => text.includes(`"${k}"`)), hint), title, subject, details: rest && rest !== 'null' ? [['', rest]] : [], raw: '' };
}

// An approval's detail in words. A chat agent's detail is its raw input as JSON, often cut at 300 characters with "…"; when it reads
// as an object (or a cut one still names its command or file) the window shows that thing. Anything else is shown exactly as it came.
export function detailText(detail: string): string {
  const s = detail.trim();
  if (!s.startsWith('{')) return detail;
  let parsed: unknown;
  try { parsed = JSON.parse(s); } catch { parsed = undefined; }
  if (isObject(parsed)) {
    const { subject } = pickSubject(parsed);
    if (subject) return subject;
    const plain = Object.entries(parsed).filter(([, v]) => typeof v === 'string' || typeof v === 'number' || typeof v === 'boolean');
    return plain.length ? plain.slice(0, 3).map(([k, v]) => `${k}: ${String(v)}`).join(' · ') : detail;
  }
  for (const [key] of SUBJECTS) {
    const v = stringField(s, key);
    if (v) return v.split('\n')[0] as string;
  }
  return detail;
}

// Whether the title already says the subject, so it is not shown twice ("Read src/a.rs" and "src/a.rs").
export function titleSays(title: string, subject: string): boolean {
  if (!title || !subject) return false;
  if (title.includes(subject)) return true;
  const base = subject.split(/[\\/]/).pop() ?? '';
  return base.length > 2 && title.includes(base);
}
