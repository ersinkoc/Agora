// The "/" menu of the composer (ADR-0090): the commands the agent offers (`AgentCommands`). Choosing one only puts `/name ` in the box;
// what is sent is the user's own prompt text through the usual prompt, so the agent runs it as any other message. A command's name,
// description and hint are the agent's own words: untrusted, shown as text. Pure, so it is tested without a page.
import type { CommandModel } from '../model';

export const MAX_SHOWN = 8;

/** The text after a leading "/" while the caret is still inside that first word; undefined when the box is not asking for a command. */
export function slashQuery(text: string, caret: number): string | undefined {
  const head = text.slice(0, caret);
  const m = /^\/([^\s/]*)$/.exec(head);
  return m ? (m[1] as string) : undefined;
}

const bare = (name: string): string => name.replace(/^\/+/, '');

/** The commands whose name starts with `query`, then those that contain it or whose description does (case-insensitive), in the agent's order. */
export function filterCommands(list: readonly CommandModel[], query: string, max = MAX_SHOWN): CommandModel[] {
  const q = query.toLowerCase();
  const name = (c: CommandModel) => bare(c.name).toLowerCase();
  const starts = list.filter((c) => name(c).startsWith(q));
  const rest = list.filter((c) => !name(c).startsWith(q) && (name(c).includes(q) || c.description.toLowerCase().includes(q)));
  return [...starts, ...rest].slice(0, max);
}

/** What choosing a command puts in the box: `/name ` (the person adds the rest). */
export const insertText = (c: CommandModel): string => `/${bare(c.name)} `;

/** The box after choosing a command: only the "/token" the caret is in is replaced (with `/name `); what the person typed after it stays. */
export function applyCommand(text: string, caret: number, c: CommandModel): { text: string; caret: number } {
  const rest = text.slice(caret).replace(/^\S*/, ''); // the rest of the word the caret is in belongs to the token
  const head = `/${bare(c.name)}`;
  const next = head + (rest.startsWith(' ') || rest.startsWith('\n') ? '' : ' ') + rest;
  return { text: next, caret: head.length + 1 };
}

/** The commands without a repeated name (the first wins), so a list never has two rows that would act as one. */
export function uniqueCommands(list: readonly CommandModel[]): CommandModel[] {
  const seen = new Set<string>();
  return list.filter((c) => { const n = bare(c.name).toLowerCase(); if (!n || seen.has(n)) return false; seen.add(n); return true; });
}

/** Where the highlight goes after an arrow key, wrapping at both ends. */
export const stepIndex = (at: number, by: 1 | -1, count: number): number => (count === 0 ? 0 : (at + by + count) % count);
