// The Gallery (ADR-0120, docs/ux/themes.md and plugins.md "The gallery"): both community catalogs on one screen, with search, sorting
// and previews. Themes are drawn from their own files (fetched from the hub a few at a time, checked by the window's rules) and install
// in one step for the person; plugins show the screenshots the hub kept and the command the hub's owner runs (nothing installs a plugin
// from here). Every text from a catalog is shown as text. Popularity is the number the catalog publishes; Agora counts nothing.
import { useEffect, useMemo, useState, useSyncExternalStore } from 'react';
import { t } from '../i18n';
import { Button, SegmentedControl } from '../components/kit';
import { Tag } from '../components/kit/Pill';
import { capabilityKey } from '../plugins/capabilities';
import { type CatalogPlugin, compareVersions } from '../plugins/hub';
import { pluginStore } from '../plugins/store';
import { usePlugins } from '../plugins/usePlugins';
import { type CatalogEntry, type CatalogList, catalogSource, fetchCatalogTheme, installedOf, loadCatalog, subscribeCatalogSource } from '../themes/catalog';
import { type ThemeData, deserialise } from '../themes/file';
import { CatalogPreview } from '../themes/ThemeCatalog';
import { fullText, issueText } from '../themes/messages';
import { themeStore } from '../themes/store';
import { type Scheme, hasScheme, looksOf } from '../themes/tokens';
import { SORT_KEYS, type SortKey, defaultSort, matches, sorted } from './order';

export type GalleryTab = 'themes' | 'plugins';
/** How many theme files are fetched at once for the cards' previews. */
const PREVIEW_AT_ONCE = 3;

export function CatalogGallery({ tab, onTab, onBack }: { tab: GalleryTab; onTab: (tab: GalleryTab) => void; onBack: () => void }) {
  const ask = useSyncExternalStore(subscribeCatalogSource, catalogSource, catalogSource);
  const { catalog: plugins, plugins: installed } = usePlugins();
  const [themes, setThemes] = useState<CatalogList | undefined>();
  const [query, setQuery] = useState('');
  const [sort, setSort] = useState<SortKey | undefined>();
  useEffect(() => {
    let stop = false;
    if (ask) void loadCatalog(ask).then((l) => { if (!stop) setThemes(l); });
    return () => { stop = true; };
  }, [ask]);
  const pluginEntries = plugins?.on ? plugins.entries : [];
  const themeEntries = themes?.on ? themes.entries : [];
  const items = tab === 'themes' ? themeEntries : pluginEntries;
  const order = sort ?? defaultSort(items);
  return (
    <main className="mx-auto grid w-full max-w-6xl content-start gap-4 p-4 sm:p-6" aria-labelledby="gallery-title" data-catalog-gallery="">
      <div className="flex flex-wrap items-center gap-3">
        <Button size="sm" variant="ghost" onClick={onBack}>{t('catalog-gallery-back')}</Button>
        <h1 id="gallery-title" className="m-0 text-title font-semibold">{t('catalog-gallery-title')}</h1>
        <SegmentedControl<GalleryTab> label={t('catalog-gallery-show')} value={tab} onChange={(k) => { onTab(k); setSort(undefined); }}
          items={[{ value: 'themes', label: t('catalog-gallery-themes', { count: themeEntries.length }) }, { value: 'plugins', label: t('catalog-gallery-plugins', { count: pluginEntries.length }) }]} />
        <span className="flex-1" />
        <input type="search" className="min-w-0 flex-1 basis-full sm:basis-auto sm:max-w-64" value={query} onChange={(e) => setQuery(e.target.value)}
          placeholder={t(tab === 'themes' ? 'catalog-gallery-search-themes' : 'catalog-gallery-search-plugins')} aria-label={t(tab === 'themes' ? 'catalog-gallery-search-themes' : 'catalog-gallery-search-plugins')} />
        <label className="flex items-center gap-2 text-sm text-muted">
          {t('catalog-gallery-sort')}
          <select value={order} onChange={(e) => setSort(e.target.value as SortKey)} data-gallery-sort="">
            {SORT_KEYS.map((k) => <option key={k} value={k}>{t(`catalog-gallery-sort-${k}`)}</option>)}
          </select>
        </label>
      </div>
      <p className="m-0 text-xs text-muted">{t('catalog-gallery-intro')}</p>
      {tab === 'themes'
        ? <ThemeGrid list={themes} entries={sorted(themeEntries, order).filter((e) => matches(e, query))} ask={ask} />
        : <PluginGrid list={plugins} entries={sorted(pluginEntries, order).filter((e) => matches(e, query))}
            installed={(id) => installed.find((p) => !p.dev && p.plugin.manifest.id === id)?.plugin.manifest.version} />}
    </main>
  );
}

const Popularity = ({ value, added }: { value: number; added: string }) => (value > 0 || added
  ? <span className="flex flex-wrap gap-2 text-caption text-muted">
      {value > 0 && <span title={t('catalog-gallery-popularity-hint')} data-popularity={value}>{t('catalog-gallery-popularity', { count: value })}</span>}
      {added && <span data-added={added}>{t('catalog-gallery-added', { day: added })}</span>}
    </span>
  : null);

const Problems = ({ lines }: { lines: readonly string[] }) => (
  <ul role="alert" className="m-0 grid list-none gap-1 rounded-s border border-danger p-2 text-xs">{lines.map((l, i) => <li key={i}>{l}</li>)}</ul>
);

// ---- Themes -------------------------------------------------------------------------------------------------------------------------

type Read = { ok: true; data: string; theme: ThemeData } | { ok: false; lines: string[] };

function ThemeGrid({ list, entries, ask }: { list: CatalogList | undefined; entries: CatalogEntry[]; ask: ReturnType<typeof catalogSource> }) {
  const state = useSyncExternalStore(themeStore.subscribe, themeStore.state, themeStore.state);
  const [reads, setReads] = useState<Record<string, Read>>({});
  const [open, setOpen] = useState<string | undefined>();
  const [busy, setBusy] = useState<string | undefined>();
  const [problems, setProblems] = useState<{ id: string; lines: string[] } | undefined>();
  const wanted = useMemo(() => entries.map((e) => e.id), [entries]);
  // The cards' previews: each theme's file, a few at a time, in the order shown; kept for the visit.
  useEffect(() => {
    if (!ask) return;
    let stop = false;
    const todo = wanted.filter((id) => !reads[id]);
    const next = async (): Promise<void> => {
      const id = todo.shift();
      if (id === undefined || stop) return;
      const got = await fetchCatalogTheme(ask, id);
      if (stop) return;
      const r = got.ok ? deserialise(got.data) : undefined;
      setReads((p) => ({ ...p, [id]: got.ok && r?.ok ? { ok: true, data: got.data, theme: r.theme } : { ok: false, lines: got.ok && r && !r.ok ? [t('theme-catalog-refused'), ...r.issues.map(issueText)] : [t('theme-catalog-unavailable')] } }));
      return next();
    };
    void Promise.all(Array.from({ length: PREVIEW_AT_ONCE }, next));
    return () => { stop = true; };
    // Only a new list or a new connection asks again; a finished read must not restart the queue.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ask, wanted]);
  if (!ask || !list) return <p className="m-0 text-sm text-muted">{t('catalog-gallery-unavailable')}</p>;
  if (!list.on) return <p className="m-0 text-sm text-muted">{t('theme-catalog-off')}</p>;
  if (list.entries.length === 0) return <p className="m-0 text-sm text-muted">{t('theme-catalog-empty')}</p>;
  if (entries.length === 0) return <p className="m-0 text-sm text-muted">{t('catalog-gallery-no-match')}</p>;
  const install = (e: CatalogEntry) => {
    const r = reads[e.id];
    if (!r?.ok) { setOpen(e.id); return; }
    const have = installedOf(e, state.user);
    if (have.kind === 'current') { themeStore.choose(have.theme.id); return; }
    setBusy(e.id);
    const origin = { id: e.id, version: e.version };
    const saved = have.kind === 'older' ? themeStore.upgrade(have.theme.id, r.data, origin) : themeStore.install(r.data, origin);
    setBusy(undefined);
    setProblems(saved.ok ? undefined : { id: e.id, lines: 'full' in saved ? [fullText()] : saved.issues.map(issueText) });
  };
  const shown = open ? entries.find((e) => e.id === open) : undefined;
  return (
    <div className={`grid items-start gap-4 ${shown ? 'lg:grid-cols-[minmax(0,1fr)_380px]' : ''}`}>
      <ul className="m-0 grid list-none grid-cols-1 items-start gap-3 p-0 sm:grid-cols-[repeat(auto-fill,minmax(250px,1fr))]">
        {entries.map((e) => {
          const r = reads[e.id];
          const have = installedOf(e, state.user);
          return (
            <li key={e.id} className={`kit-card grid content-start overflow-hidden p-0 ${open === e.id ? 'ring-1 ring-accent' : ''}`} data-gallery-theme={e.id}>
              <div className="min-h-32 border-b border-line">
                {r?.ok ? <CatalogPreview theme={r.theme} scheme={hasScheme(r.theme, themeStore.scheme()) ? themeStore.scheme() : r.theme.scheme} />
                  : <div className="grid h-32 place-items-center text-xs text-muted">{r ? t('catalog-gallery-preview-refused') : t('catalog-gallery-preview-loading')}</div>}
              </div>
              <div className="grid gap-1.5 p-3">
                <div className="flex min-w-0 items-center gap-2">
                  <span className="min-w-0 flex-1 truncate font-semibold">{e.name}</span>
                  {have.kind === 'current' ? <span className="kit-pill kit-tone-finished">{t('theme-catalog-installed')}</span>
                    : have.kind === 'older' ? <span className="kit-pill kit-tone-accent" data-newer="">{t('theme-catalog-newer', { version: e.version })}</span>
                      : <Tag>{t('theme-catalog-version', { version: e.version })}</Tag>}
                </div>
                <span className="truncate text-xs text-muted">{t('theme-catalog-by', { author: e.author, licence: e.licence })}</span>
                <Popularity value={e.popularity} added={e.added} />
                <div className="flex flex-wrap gap-2">
                  <Button size="sm" variant={have.kind === 'current' ? 'secondary' : 'primary'} disabled={busy === e.id || (r !== undefined && !r.ok)} onClick={() => install(e)}>
                    {have.kind === 'current' ? t('catalog-gallery-use') : have.kind === 'older' ? t('theme-catalog-update', { version: e.version }) : t('theme-catalog-install')}
                  </Button>
                  <Button size="sm" variant="ghost" aria-expanded={open === e.id} onClick={() => setOpen(open === e.id ? undefined : e.id)}>{t('catalog-gallery-details')}</Button>
                </div>
                {problems?.id === e.id && <Problems lines={problems.lines} />}
              </div>
            </li>
          );
        })}
      </ul>
      {shown && <ThemeDetails entry={shown} read={reads[shown.id]} onClose={() => setOpen(undefined)} onInstall={() => install(shown)} installedAs={installedOf(shown, state.user)} />}
    </div>
  );
}

function ThemeDetails({ entry, read, onClose, onInstall, installedAs }: { entry: CatalogEntry; read: Read | undefined; onClose: () => void; onInstall: () => void; installedAs: ReturnType<typeof installedOf> }) {
  const [scheme, setScheme] = useState<Scheme | undefined>();
  const theme = read?.ok ? read.theme : undefined;
  const schemes = theme ? (['dark', 'light'] as const).filter((s) => !!looksOf(theme)[s]) : [];
  const shownScheme = scheme && schemes.includes(scheme) ? scheme : schemes.includes(themeStore.scheme()) ? themeStore.scheme() : schemes[0];
  return (
    <aside className="kit-card grid content-start gap-3 lg:sticky lg:top-4" aria-label={entry.name} data-gallery-details={entry.id}>
      <div className="flex items-center gap-2">
        <h2 className="m-0 min-w-0 flex-1 truncate text-title font-semibold">{entry.name}</h2>
        <Button size="sm" variant="ghost" onClick={onClose} aria-label={t('catalog-gallery-close')}>{t('catalog-gallery-close')}</Button>
      </div>
      {theme && shownScheme ? <CatalogPreview theme={theme} scheme={shownScheme} /> : read && !read.ok ? <Problems lines={read.lines} /> : <p className="m-0 text-xs text-muted">{t('catalog-gallery-preview-loading')}</p>}
      {schemes.length > 1 && shownScheme && <SegmentedControl<Scheme> label={t('catalog-gallery-scheme')} value={shownScheme} onChange={setScheme}
        items={schemes.map((s) => ({ value: s, label: t(s === 'dark' ? 'catalog-gallery-dark' : 'catalog-gallery-light') }))} />}
      <p className="m-0 text-xs text-muted">{t('theme-catalog-by', { author: entry.author, licence: entry.licence })} · {t('theme-catalog-version', { version: entry.version })}</p>
      <Popularity value={entry.popularity} added={entry.added} />
      {installedAs.kind === 'older' && <p className="m-0 text-xs text-muted">{t('catalog-gallery-you-have', { version: installedAs.from })}</p>}
      <div><Button variant={installedAs.kind === 'current' ? 'secondary' : 'primary'} disabled={!theme} onClick={onInstall}>
        {installedAs.kind === 'current' ? t('catalog-gallery-use') : installedAs.kind === 'older' ? t('theme-catalog-update', { version: entry.version }) : t('theme-catalog-install')}
      </Button></div>
      <p className="m-0 text-xs text-muted">{t('catalog-gallery-theme-install-note')}</p>
    </aside>
  );
}

// ---- Plugins ------------------------------------------------------------------------------------------------------------------------

function Screenshot({ id, index, className }: { id: string; index: number; className: string }) {
  const [shot, setShot] = useState<{ src: string; alt: string } | null | undefined>();
  useEffect(() => {
    let stop = false;
    void pluginStore.screenshot(id, index).then((s) => { if (!stop) setShot(s ?? null); });
    return () => { stop = true; };
  }, [id, index]);
  if (!shot) return <div className={`grid place-items-center bg-surface-2 text-caption text-muted ${className}`} aria-hidden>{shot === null ? '' : t('catalog-gallery-preview-loading')}</div>;
  return <img src={shot.src} alt={shot.alt} className={`block w-full bg-surface-2 object-cover object-top ${className}`} loading="lazy" decoding="async" />;
}

function PluginGrid({ list, entries, installed }: { list: ReturnType<typeof usePlugins>['catalog']; entries: CatalogPlugin[]; installed: (id: string) => string | undefined }) {
  const [open, setOpen] = useState<string | undefined>();
  if (!list) return <p className="m-0 text-sm text-muted">{t('catalog-gallery-unavailable')}</p>;
  if (!list.on) return <p className="m-0 text-sm text-muted">{t('catalog-gallery-plugins-off')}</p>;
  if (list.entries.length === 0) return <p className="m-0 text-sm text-muted">{t('plugin-catalog-none')}</p>;
  if (entries.length === 0) return <p className="m-0 text-sm text-muted">{t('catalog-gallery-no-match')}</p>;
  const shown = open ? entries.find((e) => e.id === open) : undefined;
  return (
    <div className={`grid items-start gap-4 ${shown ? 'lg:grid-cols-[minmax(0,1fr)_380px]' : ''}`}>
      <ul className="m-0 grid list-none grid-cols-1 items-start gap-3 p-0 sm:grid-cols-[repeat(auto-fill,minmax(250px,1fr))]">
        {entries.map((e) => (
          <li key={e.id} className={`kit-card grid content-start overflow-hidden p-0 ${open === e.id ? 'ring-1 ring-accent' : ''}`} data-gallery-plugin={e.id}>
            {e.screenshots.length > 0
              ? <Screenshot id={e.id} index={0} className="h-32 border-b border-line" />
              : <div className="grid h-32 place-items-center border-b border-line bg-surface-2 text-3xl font-semibold text-accent" aria-hidden>{e.name.slice(0, 1).toLocaleUpperCase()}</div>}
            <div className="grid gap-1.5 p-3">
              <div className="flex min-w-0 items-center gap-2">
                <span className="min-w-0 flex-1 truncate font-semibold">{e.name}</span>
                <PluginState entry={e} have={installed(e.id)} />
              </div>
              <span className="truncate text-xs text-muted">{t('plugin-version-by', { version: e.version, author: e.author })} · {t('plugin-licence', { licence: e.licence })}</span>
              <Popularity value={e.popularity} added={e.added} />
              <Asks entry={e} />
              <Button size="sm" variant="ghost" className="w-fit" aria-expanded={open === e.id} onClick={() => setOpen(open === e.id ? undefined : e.id)}>{t('catalog-gallery-details')}</Button>
            </div>
          </li>
        ))}
      </ul>
      {shown && <PluginDetails entry={shown} have={installed(shown.id)} onClose={() => setOpen(undefined)} />}
    </div>
  );
}

function PluginState({ entry, have }: { entry: CatalogPlugin; have: string | undefined }) {
  if (have === undefined) return <Tag>{t('catalog-gallery-plugin-not-added')}</Tag>;
  if (compareVersions(entry.version, have) > 0) return <span className="kit-pill kit-tone-accent" data-newer="">{t('catalog-gallery-plugin-newer', { version: entry.version, from: have })}</span>;
  return <span className="kit-pill kit-tone-finished">{t('catalog-gallery-plugin-added')}</span>;
}

const Asks = ({ entry }: { entry: CatalogPlugin }) => (entry.capabilities.length || entry.unknown
  ? <div className="flex flex-wrap gap-1" aria-label={t('catalog-gallery-asks')}>
      {entry.capabilities.map((c) => <Tag key={c}>{t(capabilityKey(c))}</Tag>)}
      {entry.unknown > 0 && <Tag>{t('plugin-catalog-unknown', { count: entry.unknown })}</Tag>}
    </div>
  : <p className="m-0 text-xs text-muted">{t('plugin-grant-nothing')}</p>);

function PluginDetails({ entry, have, onClose }: { entry: CatalogPlugin; have: string | undefined; onClose: () => void }) {
  const upgrade = have !== undefined && compareVersions(entry.version, have) > 0;
  const current = have !== undefined && !upgrade;
  const command = `agora hub plugins ${upgrade ? 'upgrade' : 'install'} ${entry.id}`;
  const [copied, setCopied] = useState(false);
  return (
    <aside className="kit-card grid content-start gap-3 lg:sticky lg:top-4" aria-label={entry.name} data-gallery-details={entry.id}>
      <div className="flex items-center gap-2">
        <h2 className="m-0 min-w-0 flex-1 truncate text-title font-semibold">{entry.name}</h2>
        <Button size="sm" variant="ghost" onClick={onClose} aria-label={t('catalog-gallery-close')}>{t('catalog-gallery-close')}</Button>
      </div>
      {entry.screenshots.map((alt, i) => <Screenshot key={`${entry.id}-${i}`} id={entry.id} index={i} className="max-h-60 rounded-s border border-line" />)}
      {entry.description && <p className="m-0 text-sm">{entry.description}</p>}
      <p className="m-0 text-xs text-muted">{t('plugin-version-by', { version: entry.version, author: entry.author })} · {t('plugin-licence', { licence: entry.licence })}</p>
      <Popularity value={entry.popularity} added={entry.added} />
      <div className="grid gap-1"><span className="text-xs text-muted">{t('catalog-gallery-asks')}</span><Asks entry={entry} /></div>
      {current ? <p className="m-0 text-xs text-muted">{t('catalog-gallery-plugin-added-note')}</p> : <>
        <p className="m-0 text-xs text-muted">{t(upgrade ? 'plugin-catalog-upgrade-how' : 'plugin-catalog-install-how')}</p>
        <code className="break-all rounded-xs bg-surface-2 p-2 font-mono text-caption text-muted select-all" data-catalog-command="">{command}</code>
        <Button size="sm" className="w-fit" onClick={() => { void navigator.clipboard?.writeText(command).then(() => setCopied(true), () => {}); }}>{t(copied ? 'catalog-gallery-copied' : 'catalog-gallery-copy-command')}</Button>
      </>}
    </aside>
  );
}
