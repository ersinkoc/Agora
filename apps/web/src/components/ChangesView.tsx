import { Fragment, useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { t } from '../i18n';
import { decodeText } from '../net/files';
import {
  baseName, byFolder, changedSpan, countByStatus, fileLines, gapLines, gapsOf, isTruncated, matchesFile, pairedLines, parseDiff,
  sameChange, splitRows, type DiffFile, type DiffLine, type DiffStatus, type Gap,
} from '../net/diff';
import { Icon } from './Icon';
import { Resizer } from './Resizer';

// The review screen's body (docs/ux/review.md): the changed files on the left, grouped by folder and filterable; one file at a time
// on the right, unified or side by side, with the unchanged lines between hunks folded and shown on request from the file as it is
// now. Read-only. One file is drawn at a time, and a very long one in parts, so a 512 KiB diff never becomes one huge page.

type Filter = 'all' | DiffStatus;
type Layout = 'unified' | 'split';
type FileText = string[] | 'loading' | 'failed' | 'stale';

const LAYOUT_KEY = 'agora.changes.layout';
const ROW_LIMIT = 3000; // rows drawn before "Show all"
const SPAN_LIMIT = 2000; // longest line whose changed part is marked

const readLayout = (): Layout => { try { return localStorage.getItem(LAYOUT_KEY) === 'split' ? 'split' : 'unified'; } catch { return 'unified'; } };
const saveLayout = (l: Layout) => { try { localStorage.setItem(LAYOUT_KEY, l); } catch { /* a private window keeps it for this view only */ } };

const STATUS_CLASS: Record<DiffStatus, string> = { added: 'text-finished', deleted: 'text-needs-you', modified: 'text-working', renamed: 'text-accent' };

export function StatusBadge({ status }: { status: DiffStatus }) {
  return <span className={`w-4 flex-none text-center font-mono text-[0.6875rem] font-bold ${STATUS_CLASS[status]}`} title={t(`changes-status-${status}`)}>{t(`changes-badge-${status}`)}</span>;
}

function Counts({ added, removed }: { added: number; removed: number }) {
  return <span className="flex-none font-mono text-[0.6875rem]"><span className="text-finished">+{added}</span> <span className="text-needs-you">−{removed}</span></span>;
}

export function ChangesView({ load, read }: {
  /** The workspace's diff against HEAD; with a path, the diff of that one file. */
  load: (path?: string) => Promise<string>;
  /** The file as it is now, to show unchanged lines the diff leaves out. */
  read?: (path: string) => Promise<Uint8Array>;
}) {
  const [diff, setDiff] = useState<{ files: DiffFile[]; truncated: boolean } | 'loading' | 'failed'>('loading');
  const [round, setRound] = useState(0);
  const [selected, setSelected] = useState<string | undefined>();
  const [filter, setFilter] = useState<Filter>('all');
  const [query, setQuery] = useState('');
  const [layout, setLayoutState] = useState<Layout>(readLayout);
  const [wrap, setWrap] = useState(false);
  const [listWidth, setListWidth] = useState(288);
  const [closedFolders, setClosedFolders] = useState<Set<string>>(new Set());
  // Per file path: the gaps shown (by `Gap.before`), the whole file shown, the file's current text, a file loaded on its own.
  const [opened, setOpened] = useState<Record<string, Set<number>>>({});
  const [whole, setWhole] = useState<Set<string>>(new Set());
  const [texts, setTexts] = useState<Record<string, FileText>>({});
  const [single, setSingle] = useState<Record<string, DiffFile | 'loading' | 'failed'>>({});
  const [showAll, setShowAll] = useState<Set<string>>(new Set());
  const body = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let live = true;
    setDiff('loading');
    load().then(
      (text) => {
        if (!live) return;
        const files = parseDiff(text);
        setDiff({ files, truncated: isTruncated(text) });
        setSelected((s) => (s !== undefined && files.some((f) => f.path === s) ? s : byFolder(files)[0]?.files[0]?.path));
      },
      () => { if (live) setDiff('failed'); },
    );
    setOpened({});
    setWhole(new Set());
    setTexts({});
    setSingle({});
    return () => { live = false; };
  }, [load, round]);

  const setLayout = (l: Layout) => { setLayoutState(l); saveLayout(l); };
  const files = diff === 'loading' || diff === 'failed' ? [] : diff.files;
  const counts = useMemo(() => countByStatus(files), [files]);
  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    return files.filter((f) => (filter === 'all' || f.status === filter) && (!q || f.path.toLowerCase().includes(q) || (f.oldPath ?? '').toLowerCase().includes(q)));
  }, [files, filter, query]);
  const groups = useMemo(() => byFolder(visible), [visible]);
  const order = useMemo(() => groups.flatMap((g) => g.files.map((f) => f.path)), [groups]);
  const totals = useMemo(() => files.reduce((s, f) => ({ added: s.added + f.added, removed: s.removed + f.removed }), { added: 0, removed: 0 }), [files]);

  const listed = files.find((f) => f.path === selected);
  const loaded = selected !== undefined ? single[selected] : undefined;
  const file = loaded && loaded !== 'loading' && loaded !== 'failed' ? loaded : listed;

  const select = useCallback((path: string | undefined) => {
    setSelected(path);
    body.current?.scrollTo({ top: 0 });
    if (path) document.getElementById(optionId(path))?.scrollIntoView({ block: 'nearest' });
  }, []);
  const step = (by: number) => {
    if (order.length === 0) return;
    const at = selected !== undefined ? order.indexOf(selected) : -1;
    select(order[Math.min(order.length - 1, Math.max(0, at < 0 ? 0 : at + by))]);
  };

  // The file as it is now, for its unchanged lines. Those lines belong to the diff on screen only if the file has not changed since:
  // the file is read first, then its own diff again, and a diff that differs from the one shown (a change anywhere, folded regions
  // included) marks the text stale. A change made and undone between the two reads goes unnoticed; it leaves the same file.
  // A renamed file is not read again: a diff of the new name alone cannot pair it with the old one and shows it as new, so only the
  // weaker check against the diff's own lines (`matchesFile`) applies. A path the second diff does not name at all (a folder
  // selected inside a repository lists tracked files from the repository's top) is a failed read, not a changed file.
  const fetchText = useCallback((f: DiffFile): void => {
    if (!read) return;
    const path = f.path;
    setTexts((x) => (x[path] && x[path] !== 'failed' ? x : { ...x, [path]: 'loading' }));
    read(path)
      .then(async (bytes): Promise<FileText> => {
        const text = decodeText(bytes);
        if (text === undefined) return 'failed';
        if (f.status === 'renamed') return fileLines(text);
        const now = parseDiff(await load(path)).find((x) => x.path === path);
        if (!now) return 'failed';
        return sameChange(f, now) ? fileLines(text) : 'stale';
      })
      .then((v) => setTexts((x) => ({ ...x, [path]: v })), () => setTexts((x) => ({ ...x, [path]: 'failed' })));
  }, [read, load]);
  // Also checked against the diff's own lines, which needs no second read.
  const textOf = (f: DiffFile): FileText | undefined => {
    const x = texts[f.path];
    return Array.isArray(x) && !matchesFile(f, x) ? 'stale' : x;
  };
  const openGap = (f: DiffFile, gap: Gap) => {
    setOpened((o) => ({ ...o, [f.path]: new Set([...(o[f.path] ?? []), gap.before]) }));
    if (texts[f.path] === undefined || texts[f.path] === 'failed') fetchText(f);
  };
  const toggleWhole = (f: DiffFile) => {
    setWhole((w) => { const n = new Set(w); if (n.has(f.path)) n.delete(f.path); else n.add(f.path); return n; });
    if (texts[f.path] === undefined || texts[f.path] === 'failed') fetchText(f);
  };
  const loadSingle = (path: string) => {
    setSingle((s) => ({ ...s, [path]: 'loading' }));
    load(path).then(
      (text) => setSingle((s) => { const f = parseDiff(text).find((x) => x.path === path); return { ...s, [path]: f && !f.partial ? f : 'failed' }; }),
      () => setSingle((s) => ({ ...s, [path]: 'failed' })),
    );
  };

  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    // The window's one-letter shortcuts (`n`, the next agent that needs you) must not act behind this view.
    if (!e.ctrlKey && !e.metaKey && !e.altKey && e.key.length === 1) e.stopPropagation();
    const el = e.target as HTMLElement;
    if (el.closest('input, textarea, select, [contenteditable="true"]') || e.altKey || e.ctrlKey || e.metaKey) return;
    if (e.key === 'j') { e.preventDefault(); step(1); }
    else if (e.key === 'k') { e.preventDefault(); step(-1); }
  };
  const onListKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === 'ArrowDown') { e.preventDefault(); step(1); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); step(-1); }
    else if (e.key === 'Home') { e.preventDefault(); select(order[0]); }
    else if (e.key === 'End') { e.preventDefault(); select(order[order.length - 1]); }
  };

  if (diff === 'loading') return <p className="muted p-4" aria-busy="true">{t('start-loading')}</p>;
  if (diff === 'failed') return <div className="p-4"><p role="alert">{t('diff-failed')}</p><button type="button" className="button" onClick={() => setRound((r) => r + 1)}>{t('changes-refresh')}</button></div>;
  if (files.length === 0) return <div className="flex items-center gap-3 p-4"><p className="muted m-0">{t('changes-none')}</p><button type="button" className="ide-iconbtn" onClick={() => setRound((r) => r + 1)} aria-label={t('changes-refresh')} title={t('changes-refresh')}><Icon name="restore" /></button></div>;

  const at = selected !== undefined ? order.indexOf(selected) : -1;
  const filters: Filter[] = ['all', 'modified', 'added', 'deleted', 'renamed'];

  return (
    <div className="flex h-full min-h-0 flex-col" onKeyDown={onKey}>
      <div className="ide-bar flex-none flex-wrap gap-y-1.5 border-b py-1.5">
        <span className="me-1 text-xs text-muted">{t('changes-summary', { count: files.length })} <Counts {...totals} /></span>
        <div role="radiogroup" aria-label={t('changes-filter')} className="flex flex-wrap items-center gap-1">
          {filters.filter((f) => f === 'all' || counts[f] > 0).map((f) => (
            <button key={f} type="button" role="radio" aria-checked={filter === f} onClick={() => setFilter(f)}
              className={`ide-chip cursor-pointer transition-colors duration-150 hover:text-ink ${filter === f ? 'border-accent text-ink' : ''}`}>
              {t(`changes-filter-${f}`)} <span className="font-mono">{counts[f]}</span>
            </button>
          ))}
        </div>
        <span className="flex-1" />
        <label className="relative flex min-w-0 items-center">
          <Icon name="search" className="pointer-events-none absolute start-2 size-3.5 text-muted" />
          <input type="search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder={t('changes-search')} aria-label={t('changes-search')}
            className="w-44 min-w-0 rounded-xs bg-surface-0 py-1 ps-7 pe-2 text-xs placeholder:text-muted" />
        </label>
        <div role="radiogroup" aria-label={t('changes-layout')} className="flex items-center rounded-xs border border-line">
          {(['unified', 'split'] as const).map((l) => (
            <button key={l} type="button" role="radio" aria-checked={layout === l} onClick={() => setLayout(l)}
              className={`px-2 py-0.5 text-xs transition-colors duration-150 ${layout === l ? 'bg-surface-2 text-ink' : 'text-muted hover:text-ink'}`}>{t(`changes-layout-${l}`)}</button>
          ))}
        </div>
        {layout === 'unified' && <button type="button" className="ide-iconbtn w-auto px-2 text-xs" aria-pressed={wrap} onClick={() => setWrap((w) => !w)}>{t('changes-wrap')}</button>}
        <button type="button" className="ide-iconbtn" onClick={() => setRound((r) => r + 1)} aria-label={t('changes-refresh')} title={t('changes-refresh')}><Icon name="restore" /></button>
      </div>
      {diff.truncated && <p role="status" className="m-0 flex-none border-b border-line bg-needs-you/10 px-3 py-1.5 text-xs text-ink">{t('changes-truncated')}</p>}

      <div className="flex min-h-0 flex-1 flex-col md:flex-row">
        <div role="listbox" tabIndex={0} aria-label={t('changes-list')} aria-activedescendant={selected !== undefined && order.includes(selected) ? optionId(selected) : undefined}
          onKeyDown={onListKey} style={{ ['--list-w' as string]: `${listWidth}px` }}
          className="max-h-44 flex-none overflow-auto border-b border-line py-1 outline-none focus-visible:ring-1 focus-visible:ring-accent md:max-h-none md:w-[var(--list-w)] md:border-b-0">
          {groups.length === 0 && <p className="muted px-3 py-2">{t('changes-no-match')}</p>}
          {groups.map((g) => {
            const closed = closedFolders.has(g.folder);
            return (
              <div key={g.folder} role="group" aria-label={g.folder || t('changes-top-folder')}>
                {g.folder !== '' && (
                  <button type="button" className="ide-row text-xs text-muted" aria-expanded={!closed} tabIndex={-1}
                    onClick={() => setClosedFolders((c) => { const n = new Set(c); if (n.has(g.folder)) n.delete(g.folder); else n.add(g.folder); return n; })}>
                    <Icon name="chevron" className={`size-3 flex-none transition-transform duration-150 ${closed ? 'rtl:rotate-180' : 'rotate-90'}`} />
                    <Icon name="folder" className="size-3.5 flex-none" />
                    <span className="truncate font-mono" dir="ltr">{g.folder}</span>
                  </button>
                )}
                {!closed && g.files.map((f) => (
                  <div key={f.path} id={optionId(f.path)} role="option" aria-selected={f.path === selected} aria-current={f.path === selected}
                    onClick={() => select(f.path)} title={f.oldPath ? `${f.oldPath} → ${f.path}` : f.path}
                    className={`ide-row cursor-pointer ${g.folder !== '' ? 'ps-7' : ''}`}>
                    <StatusBadge status={f.status} />
                    <span className={`min-w-0 flex-1 truncate font-mono text-xs ${f.status === 'deleted' ? 'text-muted line-through' : ''}`} dir="ltr">{baseName(f.path)}</span>
                    {f.partial && <span className="text-[0.6875rem] text-needs-you" title={t('changes-partial')}>…</span>}
                    {!f.binary && <Counts added={f.added} removed={f.removed} />}
                  </div>
                ))}
              </div>
            );
          })}
        </div>
        <div className="hidden md:block"><Resizer side="start" value={listWidth} min={200} max={560} onChange={setListWidth} /></div>

        <div className="flex min-h-0 min-w-0 flex-1 flex-col border-line md:border-s">
          {!file ? <p className="muted p-4">{t('changes-pick')}</p> : (
            <>
              <div className="ide-bar flex-none border-b py-1.5">
                <StatusBadge status={file.status} />
                <span className="min-w-0 flex-1 truncate font-mono text-[0.8125rem]" dir="ltr" title={file.path}>
                  {file.path.includes('/') && <span className="text-muted">{file.path.slice(0, file.path.lastIndexOf('/') + 1)}</span>}
                  <span className="font-semibold text-ink">{baseName(file.path)}</span>
                  {file.oldPath && <span className="ms-2 text-xs text-muted">{t('changes-renamed-from', { name: file.oldPath })}</span>}
                </span>
                {!file.binary && <Counts added={file.added} removed={file.removed} />}
                {read && gapsOf(file).length > 0 && <button type="button" className="ide-iconbtn w-auto px-2 text-xs" aria-pressed={whole.has(file.path)} onClick={() => toggleWhole(file)}>{t('changes-whole-file')}</button>}
                <button type="button" className="ide-iconbtn" disabled={at <= 0} onClick={() => step(-1)} aria-label={t('changes-prev')} title={`${t('changes-prev')} (k)`}><Icon name="chevron" className="-rotate-90" /></button>
                <button type="button" className="ide-iconbtn" disabled={at < 0 || at >= order.length - 1} onClick={() => step(1)} aria-label={t('changes-next')} title={`${t('changes-next')} (j)`}><Icon name="chevron" className="rotate-90" /></button>
              </div>
              <div ref={body} className="min-h-0 flex-1 overflow-auto bg-surface-0" tabIndex={-1}>
                <FileBody key={`${file.path}:${layout}`} file={file} layout={layout} wrap={wrap || layout === 'split'} text={textOf(file)}
                  opened={whole.has(file.path) ? 'all' : opened[file.path] ?? new Set()} canExpand={!!read} onOpenGap={(g) => openGap(file, g)}
                  showAll={showAll.has(file.path)} onShowAll={() => setShowAll((s) => new Set([...s, file.path]))}
                  single={selected !== undefined ? single[selected] : undefined} onLoadSingle={() => loadSingle(file.path)} onRetryText={() => fetchText(file)} />
              </div>
            </>
          )}
        </div>
      </div>
      <p className="m-0 hidden flex-none items-center gap-2 border-t border-line px-3 py-1 text-[0.6875rem] text-muted md:flex">
        <span className="ide-kbd">j</span><span className="ide-kbd">k</span>{t('changes-keys')}
      </p>
    </div>
  );
}

const optionId = (path: string) => `changes-${encodeURIComponent(path).replace(/%/g, '_')}`;

type Segment = { kind: 'lines'; lines: DiffLine[] } | { kind: 'gap'; gap: Gap };

// The file as hunks and the unchanged lines around them: shown where opened (and the file could be read and still matches), folded
// otherwise.
export function segmentsOf(file: DiffFile, opened: Set<number> | 'all', text: FileText | undefined): Segment[] {
  const gaps = new Map(gapsOf(file).map((g) => [g.before, g]));
  const lines = Array.isArray(text) ? text : undefined;
  const out: Segment[] = [];
  const add = (ls: DiffLine[]) => {
    const last = out[out.length - 1];
    if (last?.kind === 'lines') last.lines.push(...ls); else out.push({ kind: 'lines', lines: [...ls] });
  };
  const gapAt = (i: number) => {
    const g = gaps.get(i);
    if (!g) return;
    if ((opened === 'all' || opened.has(i)) && lines) {
      const ls = gapLines(g, lines);
      if (ls.length > 0) add(ls);
    } else if (g.to !== Infinity || !lines || g.from <= lines.length) {
      out.push({ kind: 'gap', gap: g });
    }
  };
  file.hunks.forEach((h, i) => {
    gapAt(i);
    const end = file.hunks[i + 1]?.at ?? file.lines.length;
    const shown = opened === 'all' && lines ? file.lines.slice(h.at + 1, end) : file.lines.slice(h.at, end);
    add(shown);
  });
  gapAt(file.hunks.length);
  return out;
}

function FileBody({ file, layout, wrap, text, opened, canExpand, onOpenGap, showAll, onShowAll, single, onLoadSingle, onRetryText }: {
  file: DiffFile; layout: Layout; wrap: boolean; text: FileText | undefined; opened: Set<number> | 'all'; canExpand: boolean;
  onOpenGap: (g: Gap) => void; showAll: boolean; onShowAll: () => void; single: DiffFile | 'loading' | 'failed' | undefined;
  onLoadSingle: () => void; onRetryText: () => void;
}) {
  const pairs = useMemo(() => pairedLines(file.lines), [file]);
  const segments = useMemo(() => segmentsOf(file, opened, text), [file, opened, text]);
  const width = `calc(${Math.max(2, String(Math.max(...file.lines.map((l) => Math.max(l.oldNo ?? 0, l.newNo ?? 0)), Array.isArray(text) ? text.length : 0)).length)}ch + 1rem)`;
  // A new or deleted file has one side only: one column, and no hunk heading (`@@ -0,0 +1,7 @@` says nothing).
  const oneSided = file.status === 'added' || file.status === 'deleted';

  const notices: ReactNode[] = [];
  if (file.partial) {
    notices.push(<div key="partial" className="flex flex-wrap items-center gap-2 border-b border-line bg-needs-you/10 px-3 py-1.5 text-xs">
      <span>{t('changes-partial')}</span>
      {single === 'loading' ? <span className="text-muted">{t('start-loading')}</span>
        : <button type="button" className="button" onClick={onLoadSingle}>{t('changes-load-file')}</button>}
      {single === 'failed' && <span role="alert" className="text-needs-you">{t('changes-load-file-failed')}</span>}
    </div>);
  }
  if (text === 'stale') notices.push(<p key="stale" role="status" className="m-0 border-b border-line bg-needs-you/10 px-3 py-1.5 text-xs">{t('changes-stale')}</p>);
  if (text === 'failed') notices.push(<p key="failed" role="alert" className="m-0 flex items-center gap-2 border-b border-line px-3 py-1.5 text-xs">{t('changes-gap-failed')}<button type="button" className="button" onClick={onRetryText}>{t('changes-retry')}</button></p>);

  if (file.binary) return <>{notices}<p className="muted p-4">{t('changes-binary')}</p></>;
  if (file.hunks.length === 0) return <>{notices}<p className="muted p-4">{t('changes-no-lines')}</p></>;

  let budget = showAll ? Infinity : ROW_LIMIT;
  let total = 0;
  for (const s of segments) total += s.kind === 'lines' ? s.lines.length : 1;
  const drawn: ReactNode[] = [];
  segments.forEach((s, i) => {
    if (budget <= 0) return;
    if (s.kind === 'gap') {
      budget--;
      const count = s.gap.to === Infinity ? undefined : s.gap.to - s.gap.from + 1;
      const label = count === undefined ? t('changes-gap-rest') : t('changes-gap', { count });
      drawn.push(canExpand && text !== 'stale'
        ? <button key={`g${i}`} type="button" onClick={() => onOpenGap(s.gap)} disabled={text === 'loading'}
            className="flex w-full items-center gap-2 border-y border-line bg-surface-2 px-3 py-1 text-start font-sans text-xs text-muted transition-colors duration-150 hover:text-ink">
            <span aria-hidden>⋯</span>{text === 'loading' ? t('start-loading') : label}
          </button>
        : <div key={`g${i}`} className="border-y border-line bg-surface-2 px-3 py-1 font-sans text-xs text-muted">⋯</div>);
      return;
    }
    const lines = (oneSided ? s.lines.filter((l) => l.kind !== 'hunk') : s.lines).slice(0, budget);
    budget -= lines.length;
    drawn.push(layout === 'split' && !oneSided
      ? <Fragment key={`s${i}`}>{splitRows(lines).map((r, j) => <SplitRowView key={j} row={r} pairs={pairs} width={width} />)}</Fragment>
      : <Fragment key={`u${i}`}>{lines.map((l, j) => <UnifiedRow key={j} line={l} pairs={pairs} width={width} />)}</Fragment>);
  });

  return (
    <>
      {notices}
      <div className={`font-mono text-xs leading-5 [tab-size:4] ${wrap ? 'w-full' : 'w-max min-w-full'}`} dir="ltr" data-wrap={wrap}>
        {drawn}
      </div>
      {budget <= 0 && !showAll && total > ROW_LIMIT && (
        <div className="p-3"><button type="button" className="button" onClick={onShowAll}>{t('changes-show-all', { count: total })}</button></div>
      )}
    </>
  );
}

const ROW_BG: Record<DiffLine['kind'], string> = { add: 'bg-finished/12', del: 'bg-needs-you/12', ctx: '', hunk: 'bg-surface-2 text-muted' };
const MARK_BG: Record<'add' | 'del', string> = { add: 'bg-finished/30', del: 'bg-needs-you/30' };

function Code({ line, pairs }: { line: DiffLine; pairs: Map<DiffLine, DiffLine> }) {
  const other = pairs.get(line);
  const span = (line.kind === 'add' || line.kind === 'del') && other && line.text.length < SPAN_LIMIT && other.text.length < SPAN_LIMIT
    ? worthMarking(line.text, changedSpan(line.text, other.text)) : undefined;
  const text = line.text || ' ';
  return (
    <>
      {span && span[1] > span[0] && (line.kind === 'add' || line.kind === 'del')
        ? <>{line.text.slice(0, span[0])}<mark className={`rounded-[2px] text-inherit ${MARK_BG[line.kind]}`}>{line.text.slice(span[0], span[1])}</mark>{line.text.slice(span[1])}</>
        : text}
      {line.noEol && <span className="ms-2 select-none font-sans text-[0.6875rem] text-muted" title={t('changes-no-eol')}>⏎̸</span>}
    </>
  );
}

// Mark the changed part of a line only when a real part of it stayed (leading indentation does not count); a line rewritten
// from scratch is shown as removed and added, without a mark over all of it.
function worthMarking(text: string, span: [number, number] | undefined): [number, number] | undefined {
  if (!span) return undefined;
  const indent = text.length - text.trimStart().length;
  const kept = Math.max(0, span[0] - indent) + (text.length - span[1]);
  return kept >= 2 && kept >= 0.3 * (text.length - indent) ? span : undefined;
}

const gutter = 'select-none px-2 text-end text-muted/70';
const codeCell = 'pe-4 whitespace-pre [[data-wrap=true]_&]:whitespace-pre-wrap [[data-wrap=true]_&]:[overflow-wrap:anywhere]';

function UnifiedRow({ line, pairs, width }: { line: DiffLine; pairs: Map<DiffLine, DiffLine>; width: string }) {
  if (line.kind === 'hunk') return <div className={`${ROW_BG.hunk} px-3 py-0.5`}>{line.text}</div>;
  return (
    <div className={`grid ${ROW_BG[line.kind]}`} style={{ gridTemplateColumns: `${width} ${width} 1.5rem minmax(0,1fr)` }}>
      <span className={gutter}>{line.oldNo ?? ''}</span>
      <span className={gutter}>{line.newNo ?? ''}</span>
      <span className={`select-none text-center ${line.kind === 'add' ? 'text-finished' : line.kind === 'del' ? 'text-needs-you' : 'text-muted'}`} aria-hidden>{line.kind === 'add' ? '+' : line.kind === 'del' ? '−' : ''}</span>
      <span className={codeCell}><Code line={line} pairs={pairs} /></span>
    </div>
  );
}

function SplitRowView({ row, pairs, width }: { row: ReturnType<typeof splitRows>[number]; pairs: Map<DiffLine, DiffLine>; width: string }) {
  if (row.hunk) return <div className={`${ROW_BG.hunk} px-3 py-0.5`}>{row.hunk.text}</div>;
  const side = (l: DiffLine | undefined, no: number | undefined) => (
    <>
      <span className={`${gutter} ${l ? ROW_BG[l.kind] : 'bg-surface-1'}`}>{no ?? ''}</span>
      <span className={`${codeCell} border-line ${l ? ROW_BG[l.kind] : 'bg-surface-1'}`}>{l ? <Code line={l} pairs={pairs} /> : ''}</span>
    </>
  );
  return (
    <div className="grid" style={{ gridTemplateColumns: `${width} minmax(0,1fr) ${width} minmax(0,1fr)` }}>
      {side(row.left, row.left?.oldNo)}
      {side(row.right, row.right?.newNo)}
    </div>
  );
}
