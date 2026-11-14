import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { t } from '../i18n';
import { baseName, parseDiff, type DiffFile, type DiffStatus } from '../net/diff';
import { ancestors, changedFolders, changeMarks, crumbs, findMatches, statusByPath, type ChangeMark, type Match } from '../net/fileview';
import { decodeText, formatSize, joinPath, type FileItem, type WorkspaceFiles } from '../net/files';
import { StatusBadge } from './ChangesView';
import { Icon } from './Icon';
import { Resizer } from './Resizer';

// An agent's workspace as an editor shows it (docs/ux/main-window.md, Files): a folder tree that opens lazily, with the files and
// folders that hold changes marked; open files as tabs; the file with line numbers, where it changed in the gutter, and a search.
// Read-only. The text is drawn a screenful at a time, so a 512 KiB file costs what its visible lines cost.

type Listing = FileItem[] | 'loading' | 'failed';
type Content = { lines: string[]; bytes: number } | 'loading' | 'binary' | 'failed';
interface Tab { path: string; size: number }

const LINE = 20; // px per line: text-xs with leading-5
const OVERSCAN = 30;
const MAX_TABS = 12;

export function FilesView({ files, load, changed, compact = false }: {
  files: WorkspaceFiles;
  /** The workspace's diff, for the change marks; absent for a folder without Git. */
  load?: (path?: string) => Promise<string>;
  /** How many files the node counts as changed (it counts every few seconds): the marks are read again when it moves, so they
   *  follow an agent at work, and a first read made before the node had the workspace is not the last. */
  changed?: number;
  /** In a pane: the tree starts narrower and the hints are left out. */
  compact?: boolean;
}) {
  const [listings, setListings] = useState<Record<string, Listing>>({});
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [active, setActive] = useState<string | undefined>();
  const [contents, setContents] = useState<Record<string, Content>>({});
  const [changes, setChanges] = useState<DiffFile[]>([]);
  const [onlyChanged, setOnlyChanged] = useState(false);
  const [query, setQuery] = useState('');
  const [treeWidth, setTreeWidth] = useState(compact ? 220 : 280);
  const [finding, setFinding] = useState(false);
  const [needle, setNeedle] = useState('');
  const [hit, setHit] = useState(0);
  const [copied, setCopied] = useState(false);
  const [round, setRound] = useState(0);
  const filterRef = useRef<HTMLInputElement>(null);
  const findRef = useRef<HTMLInputElement>(null);

  // A folder that cannot be listed is tried twice more, a little later each time (just after an agent starts, the node may not hold
  // its workspace yet); then it says so, with a way to try again.
  const list = useCallback((dir: string) => {
    setListings((l) => (l[dir] === undefined || l[dir] === 'failed' ? { ...l, [dir]: 'loading' } : l));
    const attempt = (n: number) => files.list(dir).then(
      (items) => setListings((l) => ({ ...l, [dir]: [...items].sort((a, b) => Number(b.isDir) - Number(a.isDir) || a.name.localeCompare(b.name)) })),
      () => { if (n < 2) setTimeout(() => attempt(n + 1), 1000 * (n + 1)); else setListings((l) => ({ ...l, [dir]: 'failed' })); },
    );
    attempt(0);
  }, [files]);
  useEffect(() => { setListings({}); list(''); }, [list, round]);
  // The caller may pass a new function on every render (a pane does); the diff is read on opening and when asked, not on each one.
  const loadRef = useRef(load);
  loadRef.current = load;
  const hasDiff = load !== undefined;
  // Just after an agent starts, the node can answer before it holds the workspace: a failed read, or an empty one while it counts
  // changed files, is tried again a few times, a little later each time.
  useEffect(() => {
    let live = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const attempt = (n: number) => {
      const loader = loadRef.current;
      if (!loader) { setChanges([]); return; }
      const again = () => { if (live && n < 3) timer = setTimeout(() => attempt(n + 1), 1500 * (n + 1)); };
      loader().then(
        (d) => { if (!live) return; const parsed = parseDiff(d); setChanges(parsed); if (parsed.length === 0 && (changed ?? 0) > 0) again(); },
        () => { if (live) { setChanges([]); again(); } },
      );
    };
    attempt(0);
    return () => { live = false; if (timer) clearTimeout(timer); };
  }, [hasDiff, round, changed]);
  const status = useMemo(() => statusByPath(changes), [changes]);
  const folders = useMemo(() => changedFolders(status.keys()), [status]);
  const changeOf = (path: string) => changes.find((c) => c.path === path);

  const read = useCallback((path: string) => {
    setContents((c) => ({ ...c, [path]: 'loading' }));
    files.read(path).then(
      (b) => setContents((c) => {
        const text = decodeText(b);
        if (text === undefined) return { ...c, [path]: 'binary' };
        const lines = text.split('\n').map((l) => (l.endsWith('\r') ? l.slice(0, -1) : l));
        if (lines.length > 1 && lines[lines.length - 1] === '') lines.pop();
        return { ...c, [path]: { lines, bytes: b.length } };
      }),
      () => setContents((c) => ({ ...c, [path]: 'failed' })),
    );
  }, [files]);
  const openFile = (path: string, size: number) => {
    setTabs((ts) => (ts.some((x) => x.path === path) ? ts : [...ts, { path, size }].slice(-MAX_TABS)));
    setActive(path);
    setHit(0);
    if (contents[path] === undefined || contents[path] === 'failed') read(path);
  };
  const closeTab = (path: string) => {
    const at = tabs.findIndex((x) => x.path === path);
    const next = tabs.filter((x) => x.path !== path);
    setTabs(next);
    if (active === path) setActive(next[Math.min(at, next.length - 1)]?.path);
  };
  const toggle = (path: string) => setOpen((o) => { const n = new Set(o); if (n.has(path)) n.delete(path); else n.add(path); return n; });
  // Opens the folders down to `path` so it shows in the tree, listing each one that was not read yet.
  const reveal = (path: string, andSelf = false) => {
    const dirs = [...ancestors(path), ...(andSelf ? [path] : [])];
    setOpen((o) => new Set([...o, ...dirs]));
    for (const d of dirs) if (listings[d] === undefined) list(d);
    setOnlyChanged(false);
    setQuery('');
  };

  // Every file listed so far, for the name filter (folders not opened yet are not searched; the changed files always are).
  const known = useMemo(() => {
    const out = new Map<string, number>();
    for (const [dir, l] of Object.entries(listings)) if (Array.isArray(l)) for (const f of l) if (!f.isDir) out.set(joinPath(dir, f.name), f.size);
    for (const c of changes) if (!out.has(c.path)) out.set(c.path, -1);
    return out;
  }, [listings, changes]);
  const flat = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q && !onlyChanged) return undefined;
    return [...known.entries()]
      .filter(([p]) => (!onlyChanged || status.has(p)) && (!q || p.toLowerCase().includes(q)))
      .sort(([a], [b]) => a.localeCompare(b))
      .slice(0, 500);
  }, [known, query, onlyChanged, status]);

  const content = active !== undefined ? contents[active] : undefined;
  const lines = content && typeof content === 'object' ? content.lines : undefined;
  const marks = useMemo(() => {
    const c = active !== undefined ? changes.find((x) => x.path === active) : undefined;
    return c && !c.binary ? changeMarks(c) : new Map<number, ChangeMark>();
  }, [changes, active]);
  const matches = useMemo(() => (lines && finding ? findMatches(lines, needle) : []), [lines, needle, finding]);
  const current = matches.length > 0 ? matches[Math.min(hit, matches.length - 1)] : undefined;

  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    keepLetters(e);
    const typing = !!(e.target as HTMLElement).closest('input, textarea, select');
    if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === 'f' && lines) {
      e.preventDefault();
      setFinding(true);
      requestAnimationFrame(() => findRef.current?.select());
    } else if (e.key === 'Escape' && finding) {
      e.preventDefault();
      setFinding(false);
    } else if (e.key === '/' && !typing) {
      e.preventDefault();
      filterRef.current?.focus();
    }
  };
  const copy = async () => {
    if (!lines) return;
    try {
      await navigator.clipboard.writeText(`${lines.join('\n')}\n`);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch { /* no clipboard (an insecure origin or a denied permission): the button just does nothing */ }
  };

  const activeStatus = active !== undefined ? status.get(active) : undefined;
  const activeTab = tabs.find((x) => x.path === active);

  return (
    <div className="@container h-full min-h-0" onKeyDown={onKey} data-finding={finding || undefined}>
    <div className="flex h-full min-h-0 flex-col @2xl:flex-row">
      <div className="flex max-h-56 flex-none flex-col border-b border-line @2xl:max-h-none @2xl:w-[var(--tree-w)] @2xl:border-b-0" style={{ ['--tree-w' as string]: `${treeWidth}px` }}>
        <div className="ide-bar flex-none gap-1.5 border-b py-1.5">
          <label className="relative flex min-w-0 flex-1 items-center">
            <Icon name="search" className="pointer-events-none absolute start-2 size-3.5 text-muted" />
            <input ref={filterRef} type="search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder={t('files-filter')} aria-label={t('files-filter')}
              className="w-full min-w-0 rounded-xs bg-surface-0 py-1 ps-7 pe-2 text-xs placeholder:text-muted" />
          </label>
          {(status.size > 0 || onlyChanged) && <button type="button" className="ide-iconbtn w-auto px-2 text-xs" aria-pressed={onlyChanged} onClick={() => setOnlyChanged((v) => !v)} title={t('files-only-changed')}>
            {t('files-changed-count', { count: status.size })}
          </button>}
          <button type="button" className="ide-iconbtn" onClick={() => setRound((r) => r + 1)} aria-label={t('files-refresh')} title={t('files-refresh')}><Icon name="restore" /></button>
        </div>
        <div className="min-h-0 flex-1 overflow-auto py-1" role="tree" aria-label={t('files-tree')}>
          {flat ? (
            flat.length === 0 ? <p className="muted px-3 py-2">{t('files-no-match')}</p>
              : flat.map(([path, size]) => {
                const st = status.get(path);
                const gone = st === 'deleted';
                return (
                  <button key={path} type="button" role="treeitem" aria-selected={path === active} aria-current={path === active} disabled={gone}
                    className="ide-row font-mono text-xs disabled:opacity-60" title={gone ? t('files-deleted') : path}
                    onClick={() => { if (!gone) { openFile(path, size < 0 ? 0 : size); reveal(path); } }}>
                    <Icon name="files" className="size-3.5 flex-none text-muted" />
                    <span className={`min-w-0 flex-1 truncate text-start ${gone ? 'line-through' : ''}`} dir="ltr">{path}</span>
                    {st && <StatusBadge status={st} />}
                  </button>
                );
              })
          ) : (
            <TreeLevel dir="" depth={0} listings={listings} open={open} list={list} toggle={toggle} pick={openFile} active={active} status={status} folders={folders} />
          )}
        </div>
      </div>
      <div className="hidden @2xl:block"><Resizer side="start" value={treeWidth} min={160} max={520} onChange={setTreeWidth} /></div>

      <div className="flex min-h-0 min-w-0 flex-1 flex-col border-line @2xl:border-s">
        {tabs.length === 0 ? (
          <div className="grid flex-1 place-items-center p-6 text-center">
            <div className="grid gap-2">
              <p className="muted m-0">{t('files-pick')}</p>
              {!compact && <p className="m-0 flex items-center justify-center gap-1.5 text-[0.6875rem] text-muted"><span className="ide-kbd">/</span>{t('files-key-filter')}<span className="ide-kbd ms-2">Ctrl F</span>{t('files-key-find')}</p>}
            </div>
          </div>
        ) : (
          <>
            <div role="tablist" aria-label={t('files-open-files')} className="flex flex-none overflow-x-auto border-b border-line bg-surface-1">
              {tabs.map((x) => (
                <div key={x.path} role="tab" aria-selected={x.path === active} tabIndex={x.path === active ? 0 : -1} className="ide-tab cursor-pointer gap-1.5 pe-1"
                  title={x.path} onClick={() => setActive(x.path)} onAuxClick={(e) => { if (e.button === 1) closeTab(x.path); }}
                  onKeyDown={(e) => { if (e.key === 'Delete') closeTab(x.path); }}>
                  {status.get(x.path) && <StatusBadge status={status.get(x.path) as DiffStatus} />}
                  <span className="truncate font-mono text-xs" dir="ltr">{baseName(x.path)}</span>
                  <button type="button" className="ide-iconbtn size-5 flex-none" aria-label={t('files-close-tab', { name: baseName(x.path) })}
                    onClick={(e) => { e.stopPropagation(); closeTab(x.path); }}><Icon name="close" className="size-3" /></button>
                </div>
              ))}
            </div>
            {active !== undefined && (
              <div className="ide-bar flex-none flex-wrap gap-y-1 border-b py-1">
                <nav aria-label={t('files-path')} className="flex min-w-0 flex-1 items-center gap-0.5 overflow-hidden font-mono text-xs" dir="ltr">
                  {crumbs(active).map((c, i, all) => (
                    <span key={c.path} className="flex min-w-0 items-center gap-0.5">
                      {i > 0 && <Icon name="chevron" className="size-3 flex-none text-muted" />}
                      {i < all.length - 1
                        ? <button type="button" className="truncate rounded-xs px-1 text-muted hover:bg-surface-2 hover:text-ink" onClick={() => reveal(c.path, true)}>{c.name}</button>
                        : <span className="truncate px-1 font-semibold text-ink">{c.name}</span>}
                    </span>
                  ))}
                </nav>
                <span className="hidden flex-none text-[0.6875rem] text-muted @lg:inline">
                  {lines ? t('files-lines', { count: lines.length }) : ''}{activeTab && typeof content === 'object' ? ` · ${formatSize(content.bytes)}` : ''}
                  {activeStatus && marks.size > 0 ? ` · ${t('files-changed-lines', { count: [...marks.values()].filter((m) => m !== 'del').length })}` : ''}
                </span>
                <button type="button" className="ide-iconbtn" aria-pressed={finding} disabled={!lines} onClick={() => { setFinding((f) => !f); requestAnimationFrame(() => findRef.current?.select()); }}
                  aria-label={t('files-find')} title={`${t('files-find')} (Ctrl F)`}><Icon name="search" /></button>
                <button type="button" className="ide-iconbtn" disabled={!lines} onClick={copy} aria-label={t('files-copy')} title={copied ? t('files-copied') : t('files-copy')}><Icon name="copy" /></button>
                <button type="button" className="ide-iconbtn" onClick={() => read(active)} aria-label={t('files-reload')} title={t('files-reload')}><Icon name="restore" /></button>
              </div>
            )}
            {finding && lines && (
              <div className="ide-bar flex-none gap-2 border-b py-1">
                <input ref={findRef} type="search" value={needle} autoFocus aria-label={t('files-find')} placeholder={t('files-find')}
                  onChange={(e) => { setNeedle(e.target.value); setHit(0); }}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' && matches.length > 0) { e.preventDefault(); setHit((h) => (e.shiftKey ? (h - 1 + matches.length) % matches.length : (h + 1) % matches.length)); }
                  }}
                  className="w-56 min-w-0 rounded-xs bg-surface-0 px-2 py-1 font-mono text-xs" />
                <span className="text-[0.6875rem] text-muted" aria-live="polite">{needle ? (matches.length === 0 ? t('files-find-none') : t('files-find-at', { used: Math.min(hit, matches.length - 1) + 1, total: matches.length })) : ''}</span>
                <button type="button" className="ide-iconbtn" disabled={matches.length === 0} onClick={() => setHit((h) => (h - 1 + matches.length) % matches.length)} aria-label={t('files-find-prev')}><Icon name="chevron" className="-rotate-90" /></button>
                <button type="button" className="ide-iconbtn" disabled={matches.length === 0} onClick={() => setHit((h) => (h + 1) % matches.length)} aria-label={t('files-find-next')}><Icon name="chevron" className="rotate-90" /></button>
                <span className="flex-1" />
                <button type="button" className="ide-iconbtn" onClick={() => setFinding(false)} aria-label={t('files-find-close')}><Icon name="close" className="size-3.5" /></button>
              </div>
            )}
            <div className="min-h-0 flex-1 bg-surface-0">
              {content === undefined || content === 'loading' ? <p className="muted p-4" aria-busy="true">{t('start-loading')}</p>
                : content === 'binary' ? <p className="muted p-4">{t('files-binary')}</p>
                : content === 'failed' ? <p role="alert" className="p-4">{activeStatus === 'deleted' ? t('files-deleted') : t('files-too-large')}</p>
                : <CodeView key={active} label={active ?? ''} lines={content.lines} marks={marks} matches={finding ? matches : []} current={current} />}
            </div>
            {active !== undefined && changeOf(active) && marks.size === 0 && activeStatus === 'renamed' && (
              <p className="m-0 flex-none border-t border-line px-3 py-1 text-[0.6875rem] text-muted">{t('changes-renamed-from', { name: changeOf(active)?.oldPath ?? '' })}</p>
            )}
          </>
        )}
      </div>
    </div>
    </div>
  );
}

function TreeLevel({ dir, depth, listings, open, list, toggle, pick, active, status, folders }: {
  dir: string; depth: number; listings: Record<string, Listing>; open: Set<string>; list: (dir: string) => void; toggle: (path: string) => void;
  pick: (path: string, size: number) => void; active?: string; status: Map<string, DiffStatus>; folders: Set<string>;
}) {
  const l = listings[dir];
  useEffect(() => { if (l === undefined) list(dir); }, [dir, l, list]);
  const pad = { paddingInlineStart: 8 + depth * 14 };
  if (l === undefined || l === 'loading') return <p className="muted m-0 px-2 py-1 text-xs" style={pad}>…</p>;
  if (l === 'failed') return <p role="alert" className="m-0 flex items-center gap-2 px-2 py-1 text-xs text-needs-you" style={pad}>{t('files-failed')}<button type="button" className="button" onClick={() => list(dir)}>{t('changes-retry')}</button></p>;
  if (l.length === 0 && depth === 0) return <p className="muted m-0 px-3 py-2">{t('files-empty')}</p>;
  return (
    <div role="group">
      {l.map((f) => {
        const path = joinPath(dir, f.name);
        const st = status.get(path);
        const isOpen = open.has(path);
        return (
          <div key={path}>
            <button type="button" role="treeitem" aria-expanded={f.isDir ? isOpen : undefined} aria-selected={path === active} aria-current={path === active}
              className="ide-row gap-1.5 py-0.5 font-mono text-xs" style={pad} title={path}
              onClick={() => (f.isDir ? toggle(path) : pick(path, f.size))}>
              <span className="w-3 flex-none text-muted">{f.isDir && <Icon name="chevron" className={`size-3 transition-transform duration-150 ${isOpen ? 'rotate-90' : 'rtl:rotate-180'}`} />}</span>
              <Icon name={f.isDir ? 'folder' : 'files'} className={`size-3.5 flex-none ${f.isDir ? 'text-muted' : 'text-muted/70'}`} />
              <span className={`min-w-0 flex-1 truncate text-start ${st ? 'text-ink' : ''}`} dir="ltr">{f.name}</span>
              {f.isDir && folders.has(path) && <span className="size-1.5 flex-none rounded-full bg-working" title={t('files-folder-changed')} />}
              {st && <StatusBadge status={st} />}
            </button>
            {f.isDir && isOpen && <TreeLevel dir={path} depth={depth + 1} listings={listings} open={open} list={list} toggle={toggle} pick={pick} active={active} status={status} folders={folders} />}
          </div>
        );
      })}
    </div>
  );
}

// Letters typed in this view are this view's: the window's one-letter shortcuts (`n`, the next agent that needs you) must not act
// behind it. Keys with a modifier, Escape and the like still reach the window (the dialog closes on Escape, the palette opens).
export function keepLetters(e: KeyboardEvent<HTMLElement>): void {
  if (!e.ctrlKey && !e.metaKey && !e.altKey && e.key.length === 1) e.stopPropagation();
}

const MARK_CLASS: Record<ChangeMark, string> = { add: 'bg-finished', mod: 'bg-working', del: 'bg-needs-you' };

// A file's lines with numbers and change marks, drawn a screenful at a time. Lines do not wrap; the view scrolls sideways.
function CodeView({ label, lines, marks, matches, current }: { label: string; lines: string[]; marks: Map<number, ChangeMark>; matches: Match[]; current?: Match }) {
  const box = useRef<HTMLDivElement>(null);
  const [top, setTop] = useState(0);
  const [height, setHeight] = useState(600);
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    setHeight(el.clientHeight);
    const ro = new ResizeObserver(() => setHeight(el.clientHeight));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  // The current match is scrolled into the middle of the view.
  useEffect(() => {
    const el = box.current;
    if (!el || !current) return;
    const y = current.line * LINE;
    if (y < el.scrollTop || y > el.scrollTop + el.clientHeight - LINE * 2) el.scrollTop = Math.max(0, y - el.clientHeight / 2);
  }, [current]);
  const byLine = useMemo(() => {
    const m = new Map<number, Match[]>();
    for (const x of matches) m.set(x.line, [...(m.get(x.line) ?? []), x]);
    return m;
  }, [matches]);
  const widest = useMemo(() => lines.reduce((w, l) => Math.max(w, l.length), 0), [lines]);
  const digits = Math.max(2, String(lines.length).length);
  const first = Math.max(0, Math.floor(top / LINE) - OVERSCAN);
  const last = Math.min(lines.length, Math.ceil((top + height) / LINE) + OVERSCAN);
  // A removal past the last line is shown on the last line.
  const markOf = (n: number): ChangeMark | undefined => marks.get(n) ?? (n === lines.length ? marks.get(n + 1) : undefined);
  const rows: ReactNode[] = [];
  for (let i = first; i < last; i++) {
    const text = lines[i] as string;
    const mark = markOf(i + 1);
    const found = byLine.get(i);
    rows.push(
      <div key={i} className="flex" style={{ height: LINE }}>
        <span className="sticky start-0 z-[1] flex flex-none bg-surface-0">
          <span className="select-none pe-2 ps-3 text-end text-muted/70" style={{ width: `calc(${digits}ch + 1.25rem)` }}>{i + 1}</span>
          <span className={`my-0.5 w-[3px] flex-none rounded-full ${mark ? MARK_CLASS[mark] : ''} ${mark === 'del' ? 'h-1 self-end' : ''}`} title={mark ? t(`files-mark-${mark}`) : undefined} />
        </span>
        <span className="whitespace-pre ps-3 pe-6">{found ? highlight(text, found, current) : text || ' '}</span>
      </div>,
    );
  }
  return (
    <div ref={box} role="region" aria-label={label} tabIndex={0} className="h-full overflow-auto font-mono text-xs leading-5 outline-none [tab-size:4]" dir="ltr"
      onScroll={(e) => setTop(e.currentTarget.scrollTop)}>
      <div style={{ height: lines.length * LINE, minWidth: `calc(${widest}ch + ${digits}ch + 4rem)`, position: 'relative' }}>
        <div style={{ transform: `translateY(${first * LINE}px)` }}>{rows}</div>
      </div>
    </div>
  );
}

function highlight(text: string, found: Match[], current?: Match): ReactNode {
  const out: ReactNode[] = [];
  let at = 0;
  for (const m of found) {
    if (m.start < at) continue;
    out.push(text.slice(at, m.start));
    const isCurrent = current && current.line === m.line && current.start === m.start;
    out.push(<mark key={m.start} className={`rounded-[2px] text-inherit ${isCurrent ? 'bg-accent/60 outline outline-1 outline-accent' : 'bg-accent/25'}`}>{text.slice(m.start, m.end)}</mark>);
    at = m.end;
  }
  out.push(text.slice(at));
  return out;
}
