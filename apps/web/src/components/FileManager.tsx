import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react';
import { t } from '../i18n';
import { crumbs } from '../net/fileview';
import { decodeText, formatSize, joinPath, readFailure, type FileItem, type WorkspaceFiles } from '../net/files';
import { baseName } from '../net/diff';
import { compareNames, filterEntries, kindOf, nextSort, parentOf, sortEntries, type FileKind, type Sort, type SortKey } from '../net/paneTools';
import { useHighlightedFile } from '../highlight/useHighlight';
import { Skeleton } from './EmptyState';
import { CodeView, keepLetters } from './FilesView';
import { Icon, type IconName } from './Icon';
import { visible } from '../net/hidden';

// The file manager (docs/ux/main-window.md, "File manager"): an agent's folder as a list you can sort and filter, with a folder tree, a
// breadcrumb and a preview. Read-only: what changes files (rename, move, delete, upload) needs your approval on the machine and a policy
// the node does not offer to this window yet, so those actions are shown switched off, with the reason. The node reports a name, whether
// it is a folder, a size and whether it is private; it reports no modified time, so there is no such column.

type Listing = FileItem[] | 'loading' | 'failed';
type Content = { lines: string[]; bytes: number } | 'loading' | 'binary' | 'failed' | 'secret';

const KIND_ICON: Record<FileKind, IconName> = { folder: 'folder', code: 'code', text: 'files', image: 'image', archive: 'files', config: 'files', file: 'files', private: 'lock' };
const ROW = 28; // px: one row of the list
const OVERSCAN = 12;
// The rows to draw: those in view and around it, and the one that holds the Tab stop (so it can always be reached with Tab).
export const rowIndexes = (first: number, last: number, keep: number): number[] => {
  const out: number[] = [];
  for (let i = first; i < last; i++) out.push(i);
  if (keep >= 0 && (keep < first || keep >= last)) out.push(keep);
  return out;
};
const EMPTY_MARKS = new Map<number, 'add' | 'mod' | 'del'>();
const columns: { key: SortKey; label: string; width: string }[] = [
  { key: 'name', label: 'fm-col-name', width: 'min-w-0 flex-1' },
  { key: 'kind', label: 'fm-col-kind', width: 'hidden w-28 flex-none @lg:block' },
  { key: 'size', label: 'fm-col-size', width: 'w-20 flex-none text-end' },
];

export function FileManager({ files, rootName, onTerminal, terminalWhy }: {
  files: WorkspaceFiles;
  rootName: string;
  /** Opens the agent's terminal beside; absent when there is none to open (`terminalWhy` says why). */
  onTerminal?: () => void;
  terminalWhy?: string;
}) {
  const [dir, setDir] = useState('');
  // Maps, not objects: a file or folder may be named `constructor` or `__proto__`.
  const [listings, setListings] = useState<Map<string, Listing>>(new Map());
  const [expanded, setExpanded] = useState<Set<string>>(new Set(['']));
  const [sort, setSort] = useState<Sort>({ key: 'name', descending: false });
  const [query, setQuery] = useState('');
  const [cursor, setCursor] = useState<string | undefined>();
  const [preview, setPreview] = useState<{ path: string; item: FileItem } | undefined>();
  const [content, setContent] = useState<Content>('loading');
  const [note, setNote] = useState<string | undefined>();
  const generation = useRef(0);
  const grid = useRef<HTMLDivElement>(null);
  const tree = useRef<HTMLDivElement>(null);
  const refocus = useRef(false);
  const noteTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  // The retry timers of a listing that failed: all cleared when the manager goes away, and the answers still in flight are dropped.
  const retries = useRef(new Set<ReturnType<typeof setTimeout>>());
  useEffect(() => () => { if (noteTimer.current) clearTimeout(noteTimer.current); generation.current++; for (const r of retries.current) clearTimeout(r); retries.current.clear(); }, []);
  const say = (key: string) => { setNote(key); if (noteTimer.current) clearTimeout(noteTimer.current); noteTimer.current = setTimeout(() => setNote(undefined), 2500); };

  // A folder that cannot be listed is tried twice more, a little later each time; then it says so, with a way to try again.
  const list = useCallback((path: string) => {
    const gen = generation.current;
    const put = (v: Listing) => { if (gen === generation.current) setListings((l) => new Map(l).set(path, v)); };
    setListings((l) => { const v = l.get(path); return v === undefined || v === 'failed' ? new Map(l).set(path, 'loading') : l; });
    const attempt = (n: number) => files.list(path).then(
      (items) => put(items),
      () => { if (gen !== generation.current) return; if (n < 2) { const id = setTimeout(() => { retries.current.delete(id); attempt(n + 1); }, 1000 * (n + 1)); retries.current.add(id); } else put('failed'); },
    );
    attempt(0);
  }, [files]);
  useEffect(() => { if (!listings.has(dir)) list(dir); }, [dir, listings, list]);
  const refresh = () => { generation.current++; setListings(new Map()); };

  const listing = listings.get(dir);
  const entries = useMemo(() => (Array.isArray(listing) ? filterEntries(sortEntries(listing, sort), query) : []), [listing, sort, query]);
  const rowPath = (e: FileItem) => joinPath(dir, e.name);
  const index = useMemo(() => new Map(entries.map((e, i) => [joinPath(dir, e.name), i])), [entries, dir]);
  const focusIndex = cursor !== undefined && index.has(cursor) ? (index.get(cursor) as number) : entries.length > 0 ? 0 : -1;
  const focusPath = focusIndex >= 0 ? rowPath(entries[focusIndex] as FileItem) : undefined;
  const cursorItem = focusIndex >= 0 ? entries[focusIndex] : undefined;
  // After the list of a new folder arrives, the first row takes the focus (when the focus was in the list). An empty folder ends the wish
  // too, so a later refresh does not take the focus.
  useEffect(() => {
    if (!refocus.current || !Array.isArray(listing)) return;
    refocus.current = false;
    grid.current?.querySelector<HTMLElement>('[role=row][data-path]')?.focus();
  }, [entries, listing]);

  // Only the rows in view are drawn (fixed height), so 5 000 entries cost what a screenful costs; the row that holds the Tab stop is
  // always drawn, and the keys move by index and scroll the row into view.
  const scroller = useRef<HTMLDivElement>(null);
  const [view, setView] = useState({ top: 0, height: 600 });
  useEffect(() => {
    const el = scroller.current;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => setView((v) => (v.height === el.clientHeight || el.clientHeight === 0 ? v : { ...v, height: el.clientHeight })));
    ro.observe(el);
    return () => ro.disconnect();
  }, [listing]);
  const first = Math.max(0, Math.floor(view.top / ROW) - OVERSCAN);
  const last = Math.min(entries.length, Math.ceil((view.top + view.height) / ROW) + OVERSCAN);
  const toFocus = useRef<string | undefined>(undefined);
  useEffect(() => {
    const p = toFocus.current;
    if (p === undefined) return;
    toFocus.current = undefined;
    [...(grid.current?.querySelectorAll<HTMLElement>('[role=row][data-path]') ?? [])].find((r) => r.dataset.path === p)?.focus();
  });
  const focusAt = (i: number) => {
    const to = Math.max(0, Math.min(entries.length - 1, i));
    const e = entries[to];
    const el = scroller.current;
    if (!e || !el) return;
    const top = to * ROW;
    if (top < el.scrollTop) el.scrollTop = top; else if (top + ROW > el.scrollTop + el.clientHeight) el.scrollTop = top + ROW - el.clientHeight;
    setView((v) => ({ ...v, top: el.scrollTop }));
    toFocus.current = rowPath(e);
    setCursor(rowPath(e));
  };

  const go = (path: string, keepFocus = false) => {
    refocus.current = keepFocus;
    setDir(path); setQuery(''); setPreview(undefined); setCursor(undefined); setView((v) => ({ ...v, top: 0 }));
    setExpanded((o) => { const n = new Set(o); n.add(''); for (let p = path; p; p = parentOf(p)) n.add(p); return n; });
  };

  // The file shown in the preview: read once per opening; a read that completes after another file was chosen is dropped.
  const reading = useRef(0);
  const openFile = (item: FileItem) => {
    const path = joinPath(dir, item.name);
    setPreview({ path, item });
    setCursor(path);
    if (item.secret) { setContent('secret'); return; }
    setContent('loading');
    const token = ++reading.current;
    files.read(path).then(
      (b) => {
        if (token !== reading.current) return;
        const text = decodeText(b);
        if (text === undefined) { setContent('binary'); return; }
        const lines = text.split('\n').map((l) => (l.endsWith('\r') ? l.slice(0, -1) : l));
        if (lines.length > 1 && lines[lines.length - 1] === '') lines.pop();
        setContent({ lines, bytes: b.length });
      },
      (e) => { if (token === reading.current) setContent(readFailure(e)); },
    );
  };
  const open = (item: FileItem, keepFocus = false) => (item.isDir ? go(rowPath(item), keepFocus) : openFile(item));
  // The preview takes the focus when it opens (in a narrow pane it replaces the list, so the row that had it is gone) and gives it back
  // to the file's row when it closes.
  const previewBox = useRef<HTMLElement>(null);
  const shown = preview?.path;
  useEffect(() => { if (shown !== undefined) previewBox.current?.focus(); }, [shown]);
  const closePreview = () => {
    const was = preview?.path;
    setPreview(undefined);
    requestAnimationFrame(() => {
      const rows = [...(grid.current?.querySelectorAll<HTMLElement>('[role=row][data-path]') ?? [])];
      (rows.find((r) => r.dataset.path === was) ?? rows.find((r) => r.tabIndex === 0))?.focus();
    });
  };
  const html = useHighlightedFile(preview?.path, typeof content === 'object' ? content.lines : undefined);

  // "/" goes to the filter, from the list and the tree alike (the one-letter keys are otherwise this view's, so the window's `n` stays out).
  const slash = (e: KeyboardEvent<HTMLElement>): boolean => {
    if (e.key !== '/' || (e.target as HTMLElement).closest('input, textarea')) return false;
    e.preventDefault(); e.stopPropagation(); document.getElementById('fm-filter')?.focus();
    return true;
  };
  const onGridKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (slash(e)) return;
    keepLetters(e);
    if ((e.target as HTMLElement).closest('input, button')) return;
    const at = index.get((document.activeElement as HTMLElement | null)?.dataset?.path ?? '') ?? -1;
    const move = (i: number) => { if (entries.length > 0) { e.preventDefault(); focusAt(i); } };
    const page = Math.max(1, Math.floor(view.height / ROW) - 1);
    const item = at >= 0 ? entries[at] : undefined;
    if (e.key === 'PageDown') move(at + page);
    else if (e.key === 'PageUp') move(at - page);
    else if (e.key === 'ArrowDown') move(at + 1);
    else if (e.key === 'ArrowUp') move(at - 1);
    else if (e.key === 'Home') move(0);
    else if (e.key === 'End') move(entries.length - 1);
    else if (e.key === 'Enter' && item) { e.preventDefault(); open(item, true); }
    else if (e.key === 'ArrowRight' && item?.isDir) { e.preventDefault(); open(item, true); }
    else if ((e.key === 'ArrowLeft' || e.key === 'Backspace') && dir) { e.preventDefault(); go(parentOf(dir), true); }
  };
  // Escape: first the preview, then the filter; the window's own Escape (closing a panel) is left alone when there is nothing to do.
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== 'Escape') return;
    if (preview) { e.preventDefault(); e.stopPropagation(); closePreview(); }
    else if (query) { e.preventDefault(); e.stopPropagation(); setQuery(''); }
  };

  // The tree is one Tab stop; arrows walk the folders shown, Right and Left open and close one (or go in and out), Enter opens it in the list.
  const onTreeKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (slash(e)) return;
    keepLetters(e);
    const items = [...(tree.current?.querySelectorAll<HTMLElement>('[role=treeitem]') ?? [])];
    const at = items.indexOf(document.activeElement as HTMLElement);
    if (at < 0) return;
    const el = items[at] as HTMLElement;
    const level = Number(el.getAttribute('aria-level') ?? 1);
    const to = (i: number) => { const x = items[Math.max(0, Math.min(items.length - 1, i))]; if (x) { e.preventDefault(); x.focus(); } };
    if (e.key === 'ArrowDown') to(at + 1);
    else if (e.key === 'ArrowUp') to(at - 1);
    else if (e.key === 'Home') to(0);
    else if (e.key === 'End') to(items.length - 1);
    else if (e.key === 'ArrowRight') {
      if (el.getAttribute('aria-expanded') === 'false') { e.preventDefault(); toggle(el.dataset.path ?? ''); }
      else if (Number(items[at + 1]?.getAttribute('aria-level') ?? 0) === level + 1) to(at + 1);
    } else if (e.key === 'ArrowLeft') {
      if (el.getAttribute('aria-expanded') === 'true' && el.dataset.path) { e.preventDefault(); toggle(el.dataset.path); return; }
      for (let i = at - 1; i >= 0; i--) if (Number(items[i]?.getAttribute('aria-level') ?? 1) < level) { to(i); return; }
    }
  };
  const toggle = (path: string) => setExpanded((o) => { const n = new Set(o); if (n.has(path)) n.delete(path); else n.add(path); return n; });

  const copyPath = async () => {
    try { await navigator.clipboard.writeText(focusPath ?? (dir || '.')); say('fm-copied'); } catch { say('fm-copy-failed'); }
  };
  const download = async (item: FileItem) => {
    try {
      const bytes = await files.read(joinPath(dir, item.name));
      const url = URL.createObjectURL(new Blob([bytes as BlobPart]));
      const a = document.createElement('a');
      a.href = url; a.download = item.name; a.click();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
    } catch { say('fm-download-failed'); }
  };
  const target = preview?.item ?? (cursorItem && !cursorItem.isDir ? cursorItem : undefined);
  const canDownload = !!target && !target.secret;
  const why = t('fm-write-why');

  return (
    <div className="@container flex h-full min-h-0 flex-col bg-surface-0" onKeyDown={onKey} aria-label={t('layout-panel-manager')} role="group">
      <div className="ide-bar flex-none flex-wrap gap-x-2 gap-y-1 border-b py-1">
        <nav aria-label={t('files-path')} className="flex min-w-0 flex-1 basis-40 items-center gap-0.5 overflow-hidden text-xs" dir="ltr">
          {[{ name: rootName, path: '' }, ...crumbs(dir)].map((c, i, all) => (
            <span key={c.path} className="flex min-w-0 items-center gap-0.5">
              {i > 0 && <Icon name="chevron" className="size-3 flex-none text-muted" />}
              {i < all.length - 1
                ? <button type="button" className="truncate rounded-xs px-1 text-muted hover:bg-surface-2 hover:text-ink" onClick={() => go(c.path)}>{visible(c.name)}</button>
                : <span aria-current="page" className="truncate px-1 font-semibold text-ink">{visible(c.name)}</span>}
            </span>
          ))}
        </nav>
        <button type="button" className="ide-iconbtn" disabled={!dir} onClick={() => go(parentOf(dir))} aria-label={t('fm-up')} title={t('fm-up')}><Icon name="chevron" className="-rotate-90" /></button>
        <button type="button" className="ide-iconbtn" onClick={refresh} aria-label={t('files-refresh')} title={t('files-refresh')}><Icon name="restore" /></button>
        <label className="relative flex min-w-36 max-w-56 flex-1 items-center">
          <Icon name="search" className="pointer-events-none absolute start-2 size-3.5 text-muted" />
          <input id="fm-filter" type="search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder={t('files-filter')} aria-label={t('files-filter')}
            className="w-full min-w-0 rounded-xs bg-surface-0 py-1 ps-7 pe-2 text-xs placeholder:text-muted" />
        </label>
      </div>
      <div className="ide-bar flex-none flex-wrap gap-1 border-b py-1" role="toolbar" aria-label={t('fm-actions')}>
        <button type="button" className="ide-iconbtn" onClick={() => void copyPath()} aria-label={t('fm-copy-path')} title={t('fm-copy-path-hint')}><Icon name="copy" /></button>
        <button type="button" className="ide-iconbtn" disabled={!canDownload} onClick={() => target && void download(target)} aria-label={t('fm-download')} title={canDownload ? t('fm-download') : t('fm-download-none')}><Icon name="download" /></button>
        {onTerminal
          ? <button type="button" className={`ide-iconbtn inline-flex! w-auto gap-1.5 px-2 text-xs ${dir ? 'cursor-not-allowed opacity-40' : ''}`} onClick={dir ? undefined : onTerminal} aria-disabled={dir ? true : undefined} aria-label={t('fm-terminal')} title={dir ? t('fm-terminal-subfolder') : t('fm-terminal-root')}><Icon name="terminal" /><span className="hidden @md:inline">{t('fm-terminal')}</span></button>
          : <button type="button" className="ide-iconbtn inline-flex! w-auto gap-1.5 px-2 text-xs opacity-40" aria-disabled="true" aria-label={t('fm-terminal')} title={t(terminalWhy ?? 'fm-terminal-none')}><Icon name="terminal" /><span className="hidden @md:inline">{t('fm-terminal')}</span></button>}
        <span aria-hidden className="mx-1 h-4 w-px bg-line" />
        {(['pencil', 'move', 'trash', 'upload'] as const).map((icon) => {
          const label = t(`fm-${icon === 'pencil' ? 'rename' : icon === 'move' ? 'move' : icon === 'trash' ? 'delete' : 'upload'}`);
          return <button key={icon} type="button" className="ide-iconbtn cursor-not-allowed opacity-40 hover:bg-transparent hover:text-muted" aria-disabled="true" aria-describedby="fm-why" aria-label={label} title={`${label}: ${why}`} onClick={() => say('fm-write-why')}><Icon name={icon} /></button>;
        })}
        <span id="fm-why" className="sr-only">{why}</span>
        <span className="flex-1" />
        <span role="status" className="text-xs text-muted">{note ? t(note) : ''}</span>
      </div>

      <div className="flex min-h-0 flex-1 flex-col @2xl:flex-row">
        <div ref={tree} role="tree" aria-label={t('fm-folders')} onKeyDown={onTreeKey} className="hidden max-h-40 flex-none overflow-auto border-b border-line py-1 @2xl:block @2xl:max-h-none @2xl:w-48 @2xl:border-b-0 @2xl:border-e">
          <TreeNode path="" name={rootName} depth={0} listings={listings} expanded={expanded} list={list} toggle={toggle} current={dir} go={go} />
        </div>

        <div className={`min-h-0 min-w-0 flex-1 flex-col ${preview ? 'hidden @4xl:flex' : 'flex'}`}>
          <div ref={grid} role="grid" aria-label={t('fm-list')} aria-rowcount={entries.length + 1} aria-busy={listing === 'loading'} onKeyDown={onGridKey} className="flex min-h-0 flex-1 flex-col">
            <div role="row" className="flex flex-none items-center gap-2 border-b border-line bg-surface-1 px-3 py-1 text-xs text-muted">
              {columns.map((c) => (
                <div key={c.key} role="columnheader" aria-sort={sort.key === c.key ? (sort.descending ? 'descending' : 'ascending') : 'none'} className={c.width}>
                  <button type="button" className={`inline-flex items-center gap-1 rounded-xs hover:text-ink ${sort.key === c.key ? 'text-ink' : ''}`} onClick={() => setSort((s) => nextSort(s, c.key))}>
                    {t(c.label)}{sort.key === c.key && <Icon name="chevronDown" className={`size-3 ${sort.descending ? '' : 'rotate-180'}`} />}
                  </button>
                </div>
              ))}
            </div>
            <div ref={scroller} className="min-h-0 flex-1 overflow-auto" onScroll={(e) => { const top = e.currentTarget.scrollTop; setView((v) => (Math.abs(v.top - top) < 1 ? v : { ...v, top })); }}>
              {listing === undefined || listing === 'loading' ? <div className="grid gap-1.5 p-3" aria-hidden>{[0, 1, 2, 3, 4].map((i) => <Skeleton key={i} className="h-4" />)}</div>
                : listing === 'failed' ? <p role="alert" className="m-0 flex items-center gap-2 p-4 text-danger">{t('files-failed')}<button type="button" className="button" onClick={() => { generation.current++; setListings((l) => { const n = new Map(l); n.delete(dir); return n; }); }}>{t('changes-retry')}</button></p>
                : listing.length === 0 ? <p className="muted m-0 p-4">{t('fm-empty')}</p>
                : entries.length === 0 ? <p className="muted m-0 p-4">{t('files-no-match')}</p>
                : <div className="relative" style={{ height: entries.length * ROW }}>{rowIndexes(first, last, focusIndex).map((i) => {
                  const e = entries[i] as FileItem;
                  const path = rowPath(e);
                  const kind = kindOf(e);
                  return (
                    <div key={e.name} role="row" aria-rowindex={i + 2} aria-selected={path === preview?.path || undefined} data-path={path} tabIndex={path === focusPath ? 0 : -1} style={{ position: 'absolute', insetInline: 0, top: i * ROW, height: ROW }}
                      aria-label={e.secret ? t('files-secret-row', { name: visible(e.name) }) : undefined}
                      className={`fm-row flex cursor-pointer items-center gap-2 px-3 text-[0.8125rem] outline-none ${path === preview?.path ? 'font-semibold' : 'hover:bg-surface-2'}`}
                      onFocus={() => setCursor((c) => (c === path ? c : path))} onClick={() => open(e)}>
                      <div role="gridcell" className="flex min-w-0 flex-1 items-center gap-2">
                        <Icon name={KIND_ICON[kind]} className={`size-3.5 ${kind === 'folder' ? 'text-accent' : 'text-muted'}`} />
                        <span className="min-w-0 truncate" dir="ltr" title={visible(e.name)}>{visible(e.name)}</span>
                      </div>
                      <div role="gridcell" className="hidden w-28 flex-none truncate text-xs text-muted @lg:block">{t(`fm-kind-${kind}`)}</div>
                      <div role="gridcell" className="w-20 flex-none text-end text-xs text-muted">{e.isDir ? '—' : formatSize(e.size)}</div>
                    </div>
                  );
                })}</div>}
            </div>
          </div>
          <p className="m-0 flex-none border-t border-line px-3 py-1 text-xs text-muted" aria-live="polite">{Array.isArray(listing) ? t('fm-count', { count: entries.length }) : ''}</p>
        </div>

        {preview && (
          <section ref={previewBox} tabIndex={-1} aria-label={t('fm-preview')} className="outline-none flex min-h-0 min-w-0 flex-1 flex-col border-line @4xl:border-s">
            <div className="ide-bar flex-none gap-1.5 border-b py-1">
              <button type="button" className="ide-iconbtn @4xl:hidden" onClick={() => closePreview()} aria-label={t('fm-back')} title={t('fm-back')}><Icon name="chevron" className="rotate-180 rtl:rotate-0" /></button>
              <span className="min-w-0 flex-1 truncate font-mono text-xs font-semibold" dir="ltr" title={visible(preview.path)}>{visible(baseName(preview.path))}</span>
              <span className="flex-none text-xs text-muted">{formatSize(preview.item.size)}</span>
              <button type="button" className="ide-iconbtn" onClick={() => closePreview()} aria-label={t('fm-close-preview')} title={t('fm-close-preview')}><Icon name="close" className="size-3.5" /></button>
            </div>
            <div className="min-h-0 flex-1">
              {content === 'loading' ? <p className="muted p-4" aria-busy="true">{t('start-loading')}</p>
                : content === 'binary' ? <p className="muted p-4">{t('fm-binary')}</p>
                : content === 'secret' ? <p role="note" className="flex items-center gap-2 p-4 text-muted"><Icon name="lock" className="size-4" />{t('files-secret')}</p>
                : content === 'failed' ? <p role="alert" className="p-4">{t('files-too-large')}</p>
                : <CodeView key={preview.path} label={preview.path} lines={content.lines} html={html} marks={EMPTY_MARKS} matches={[]} />}
            </div>
          </section>
        )}
      </div>
    </div>
  );
}

// One folder of the tree and, when open, the folders in it (the files stay in the list).
function TreeNode({ path, name, depth, listings, expanded, list, toggle, current, go }: {
  path: string; name: string; depth: number; listings: Map<string, Listing>; expanded: Set<string>; list: (p: string) => void;
  toggle: (p: string) => void; current: string; go: (p: string) => void;
}) {
  const open = expanded.has(path);
  const l = listings.get(path);
  useEffect(() => { if (open && l === undefined) list(path); }, [open, l, list, path]);
  const kids = Array.isArray(l) ? l.filter((x) => x.isDir).sort((a, b) => compareNames(a.name, b.name)) : [];
  return (
    <div role="none">
      <div role="treeitem" aria-level={depth + 1} aria-expanded={open} aria-selected={path === current} data-path={path} tabIndex={path === current ? 0 : -1}
        className={`fm-row flex min-h-7 cursor-pointer items-center gap-1.5 pe-2 text-[0.8125rem] outline-none ${path === current ? 'font-semibold' : 'hover:bg-surface-2'}`}
        style={{ paddingInlineStart: 8 + depth * 12 }} onClick={() => go(path)}
        onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); go(path); } }}>
        <button type="button" tabIndex={-1} aria-hidden className="grid size-4 flex-none place-items-center rounded-xs text-muted hover:text-ink" onClick={(e) => { e.stopPropagation(); toggle(path); }}>
          <Icon name="chevron" className={`size-3 transition-transform duration-150 ${open ? 'rotate-90' : 'rtl:rotate-180'}`} />
        </button>
        <Icon name="folder" className="size-3.5 text-muted" />
        <span className="min-w-0 truncate" dir="ltr">{visible(name)}</span>
      </div>
      {open && (l === 'loading' ? <p role="none" className="muted m-0 py-1 text-xs" style={{ paddingInlineStart: 36 + depth * 12 }}>…</p>
        : l === 'failed' ? <p role="none" className="m-0 py-1 text-xs text-danger" style={{ paddingInlineStart: 36 + depth * 12 }}>{t('files-failed')}</p>
        : <div role="group">{kids.map((k) => <TreeNode key={k.name} path={joinPath(path, k.name)} name={k.name} depth={depth + 1} listings={listings} expanded={expanded} list={list} toggle={toggle} current={current} go={go} />)}</div>)}
    </div>
  );
}
