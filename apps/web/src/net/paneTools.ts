// The tool panes of the workspace (ADR-0068): Terminal, Changes, Files and File manager, each shown for one agent's folder. Pure helpers
// only: what a pane may show, which folder a pane offers, how the file manager sorts and names what it lists.
import type { AgentTileModel } from '../model';
import type { FileItem } from './files';
import type { ToolPanel } from '../layout/panels';

// One collator for every name comparison: building one per comparison is what made sorting thousands of names slow.
const COLLATOR = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });
export const compareNames = (a: string, b: string): number => COLLATOR.compare(a, b);

// Which panels are tools is the panel registry's to say (layout/panels.ts).
export { TOOL_PANELS, isToolPanel, type ToolPanel } from '../layout/panels';
// The panels that need a workspace (a folder the node holds); Terminal needs only the agent's terminal.
export const needsFolder = (p: string): boolean => p === 'changes' || p === 'files' || p === 'manager';

// The key of the folder an agent works in: agents that share a folder share one entry.
export const folderKey = (a: AgentTileModel): string => (a.workspaceRef ? `${a.workspaceRef.node}/${a.workspaceRef.id}` : a.id);

// The right panel's Changes: one row per folder with uncommitted changes (agents that share a folder share its row, and its changes
// belong to the folder, not to one of them: ADR-0052), only the chosen project's when one is chosen, the most changed first.
export function changedFolders(fleet: readonly AgentTileModel[], projectId?: string): AgentTileModel[] {
  const byKey = new Map<string, AgentTileModel>();
  for (const a of fleet) if ((a.filesChanged ?? 0) > 0 && (!projectId || a.projectId === projectId)) byKey.set(folderKey(a), a);
  return [...byKey.values()].sort((a, b) => (b.filesChanged ?? 0) - (a.filesChanged ?? 0));
}

export interface FolderChoice { key: string; agent: AgentTileModel; changed: number }
export interface FolderGroup { project: string; folders: FolderChoice[] }

// The folders a Changes, Files or File manager pane can be pointed at, grouped by project: one per folder (a running agent stands for
// its folder before a closed one), those with changes first when `changesFirst`. Agents the node reports no folder for are left out.
export function folderChoices(fleet: readonly AgentTileModel[], changesFirst = false): FolderGroup[] {
  const byKey = new Map<string, AgentTileModel>();
  for (const a of fleet) {
    if (!a.workspaceRef) continue;
    const key = folderKey(a);
    const had = byKey.get(key);
    if (!had || (had.closed && !a.closed)) byKey.set(key, a);
  }
  const groups = new Map<string, FolderChoice[]>();
  for (const [key, agent] of byKey) {
    const project = agent.project || agent.workspace || agent.machine;
    const list = groups.get(project) ?? [];
    list.push({ key, agent, changed: agent.filesChanged ?? 0 });
    groups.set(project, list);
  }
  return [...groups.entries()]
    .map(([project, folders]) => ({ project, folders: folders.sort((a, b) => (changesFirst ? b.changed - a.changed : 0) || a.agent.machine.localeCompare(b.agent.machine) || a.agent.workspace.localeCompare(b.agent.workspace)) }))
    .sort((a, b) => (changesFirst ? b.folders.reduce((s, f) => s + f.changed, 0) - a.folders.reduce((s, f) => s + f.changed, 0) : 0) || a.project.localeCompare(b.project));
}

// The agent a tool opened from "here" is about: the one in the pane in focus, when it has a folder (or a terminal, for Terminal).
export function toolTarget(panel: ToolPanel, focused: AgentTileModel | undefined): string | undefined {
  if (!focused) return undefined;
  if (panel === 'terminal') return focused.terminal === false ? undefined : focused.id;
  return focused.workspaceRef ? focused.id : undefined;
}

// With nothing in focus to go by, a tool opens on the one thing it could show when there is exactly one (a single folder, a single
// live terminal); with several it asks.
export function soleTarget(panel: ToolPanel, fleet: readonly AgentTileModel[]): string | undefined {
  if (panel === 'terminal') { const live = fleet.filter((a) => !a.closed && a.terminal !== false); return live.length === 1 ? live[0]?.id : undefined; }
  const folders = folderChoices(fleet).flatMap((g) => g.folders);
  return folders.length === 1 ? folders[0]?.agent.id : undefined;
}

// What kind a name is, for its icon and its words. By extension only: the node reports no media types.
export type FileKind = 'folder' | 'code' | 'text' | 'image' | 'archive' | 'config' | 'file' | 'private';
const EXT: Record<string, FileKind> = {
  rs: 'code', ts: 'code', tsx: 'code', js: 'code', jsx: 'code', mjs: 'code', cjs: 'code', py: 'code', go: 'code', java: 'code', kt: 'code', c: 'code', h: 'code', cc: 'code', cpp: 'code', hpp: 'code',
  cs: 'code', rb: 'code', php: 'code', swift: 'code', sh: 'code', bash: 'code', ps1: 'code', css: 'code', scss: 'code', html: 'code', htm: 'code', sql: 'code', lua: 'code', proto: 'code',
  md: 'text', markdown: 'text', txt: 'text', rst: 'text', log: 'text', csv: 'text', tsv: 'text',
  png: 'image', jpg: 'image', jpeg: 'image', gif: 'image', webp: 'image', svg: 'image', ico: 'image', bmp: 'image', avif: 'image',
  zip: 'archive', tar: 'archive', gz: 'archive', tgz: 'archive', xz: 'archive', bz2: 'archive', '7z': 'archive', rar: 'archive',
  json: 'config', toml: 'config', yaml: 'config', yml: 'config', ini: 'config', cfg: 'config', conf: 'config', lock: 'config', xml: 'config', env: 'config',
};
export function kindOf(item: Pick<FileItem, 'name' | 'isDir' | 'secret'>): FileKind {
  if (item.isDir) return 'folder';
  if (item.secret) return 'private';
  const dot = item.name.lastIndexOf('.');
  if (dot < 0) return /^(makefile|dockerfile|license|readme|justfile)$/i.test(item.name) ? 'text' : 'file';
  return EXT[item.name.slice(dot + 1).toLowerCase()] ?? 'file';
}

export type SortKey = 'name' | 'size' | 'kind';
export interface Sort { key: SortKey; descending: boolean }
// Folders first, always; then by the chosen column, ties by name so the order never jumps.
export function sortEntries(items: readonly FileItem[], sort: Sort): FileItem[] {
  const dir = sort.descending ? -1 : 1;
  // The kind is worked out once per item, not once per comparison.
  const kinds = sort.key === 'kind' ? new Map(items.map((i) => [i, kindOf(i)])) : undefined;
  const by = (a: FileItem, b: FileItem): number => {
    if (sort.key === 'size') return a.size - b.size;
    if (kinds) return (kinds.get(a) as string).localeCompare(kinds.get(b) as string);
    return COLLATOR.compare(a.name, b.name);
  };
  return [...items].sort((a, b) => Number(b.isDir) - Number(a.isDir) || (dir * by(a, b)) || COLLATOR.compare(a.name, b.name));
}
export const nextSort = (cur: Sort, key: SortKey): Sort => (cur.key === key ? { key, descending: !cur.descending } : { key, descending: false });

export function filterEntries(items: readonly FileItem[], query: string): FileItem[] {
  const q = query.trim().toLowerCase();
  return q ? items.filter((i) => i.name.toLowerCase().includes(q)) : [...items];
}

export const parentOf = (path: string): string => (path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '');

// A folder's path for a narrow row, in two parts: what leads to the folder, which gives way first (it ends in an ellipsis), and the
// folder's own name with its separator, which stays in view (`/home/someone/Co…/login-fix`) at any width. The row's tooltip has it whole.
export function pathParts(path: string): { dir: string; name: string } {
  const sep = path.includes('\\') && !path.includes('/') ? '\\' : '/';
  const at = path.replace(/[\\/]+$/, '').lastIndexOf(sep);
  return at <= 0 ? { dir: '', name: path } : { dir: path.slice(0, at), name: path.slice(at) };
}
