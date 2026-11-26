// The catalog in the theme editor (ADR-0113 decision 6): the community themes your Agora's owner fetched, to preview and install.
// Asked for once when the editor opens (no polling), only while the window is connected; nothing is drawn while the catalog is off,
// the hub cannot say, or there is no connection. Installing copies the theme into the person's own themes, and an installed theme
// changes only when the person picks its newer version ("Update"). Every text from the catalog is shown as text.
import { useEffect, useState, useSyncExternalStore } from 'react';
import { t } from '../i18n';
import { Tag } from '../components/kit/Pill';
import { type CatalogAsk, type CatalogEntry, type CatalogList, catalogSource, fetchCatalogTheme, installedOf, loadCatalog, subscribeCatalogSource } from './catalog';
import { type ThemeData, deserialise } from './file';
import { fullText, issueText } from './messages';
import { type Saved, themeStore } from './store';
import { TerminalSample } from './TerminalSample';
import { type Look, type Scheme, hasScheme, looksOf } from './tokens';

export function ThemeCatalog() {
  const ask = useSyncExternalStore(subscribeCatalogSource, catalogSource, catalogSource);
  return ask ? <CatalogSection key="live" ask={ask} /> : null;
}

type Preview = { ok: true; theme: ThemeData } | { ok: false; lines: string[] };

// What a refused fetch means to the person.
const fetchProblem = (code: string) => t(code === 'theme.not_found' ? 'theme-catalog-gone' : code === 'theme.catalog_off' ? 'theme-catalog-off' : 'theme-catalog-unavailable');

function CatalogSection({ ask }: { ask: CatalogAsk }) {
  const [list, setList] = useState<CatalogList | undefined>();
  const [previews, setPreviews] = useState<Record<string, Preview>>({});
  const [shown, setShown] = useState<string | undefined>();
  const [busy, setBusy] = useState<string | undefined>();
  const [problems, setProblems] = useState<{ id: string; lines: string[] } | undefined>();
  const state = useSyncExternalStore(themeStore.subscribe, themeStore.state, themeStore.state);
  const scheme = themeStore.scheme();

  useEffect(() => {
    let stop = false;
    void loadCatalog(ask).then((l) => { if (!stop) setList(l); });
    return () => { stop = true; };
  }, [ask]);
  if (!list?.on) return null;

  // Fetches a theme's file and reads it with the window's own rules; the read is kept for the preview.
  const read = async (e: CatalogEntry): Promise<{ data: string; theme: ThemeData } | undefined> => {
    const got = await fetchCatalogTheme(ask, e.id);
    if (!got.ok) { setPreviews((p) => ({ ...p, [e.id]: { ok: false, lines: [fetchProblem(got.code)] } })); return undefined; }
    const r = deserialise(got.data);
    setPreviews((p) => ({ ...p, [e.id]: r.ok ? { ok: true, theme: r.theme } : { ok: false, lines: [t('theme-catalog-refused'), ...r.issues.map(issueText)] } }));
    return r.ok ? { data: got.data, theme: r.theme } : undefined;
  };
  const preview = async (e: CatalogEntry) => {
    if (shown === e.id) { setShown(undefined); return; }
    setShown(e.id);
    if (!previews[e.id]) { setBusy(e.id); await read(e); setBusy(undefined); }
  };
  const report = (id: string, r: Saved) => setProblems(r.ok ? undefined : { id, lines: 'full' in r ? [fullText()] : r.issues.map(issueText) });
  const install = async (e: CatalogEntry, into?: string) => {
    setBusy(e.id);
    setProblems(undefined);
    const got = await read(e);
    setBusy(undefined);
    if (!got) { setShown(e.id); return; }
    const origin = { id: e.id, version: e.version };
    report(e.id, into ? themeStore.upgrade(into, got.data, origin) : themeStore.install(got.data, origin));
  };

  return (
    <section className="grid gap-2" aria-labelledby="theme-catalog-title" data-theme-catalog>
      <h3 id="theme-catalog-title" className="ide-section m-0 px-0">{t('theme-catalog')}</h3>
      <p className="muted m-0 text-xs">{t('theme-catalog-hint')}</p>
      {list.entries.length === 0 ? <p className="muted m-0 text-xs">{t('theme-catalog-empty')}</p>
        : <ul className="m-0 grid list-none gap-2 p-0">
          {list.entries.map((e) => {
            const have = installedOf(e, state.user);
            const p = previews[e.id];
            return (
              <li key={e.id} className="kit-card kit-card--dense grid gap-2" data-catalog-entry={e.id}>
                <div className="flex flex-wrap items-center gap-2">
                  <div className="grid min-w-0 flex-1">
                    <span className="truncate font-semibold">{e.name}</span>
                    <span className="muted truncate text-xs">{t('theme-catalog-by', { author: e.author, licence: e.licence })}</span>
                  </div>
                  {have.kind === 'current' ? <Tag>{t('theme-catalog-installed')}</Tag>
                    : have.kind === 'older' ? <span className="kit-pill kit-tone-accent" data-newer>{t('theme-catalog-newer', { version: e.version })}</span>
                      : <Tag>{t('theme-catalog-version', { version: e.version })}</Tag>}
                </div>
                <div className="flex flex-wrap gap-2">
                  <button type="button" className="kit-btn kit-btn--sm" aria-expanded={shown === e.id} disabled={busy === e.id} onClick={() => void preview(e)}>
                    {t(shown === e.id ? 'theme-catalog-hide' : 'theme-catalog-preview')}
                  </button>
                  {have.kind === 'no' && <button type="button" className="kit-btn kit-btn--sm" disabled={busy === e.id} onClick={() => void install(e)}>{t('theme-catalog-install')}</button>}
                  {have.kind === 'older' && <button type="button" className="kit-btn kit-btn--sm" disabled={busy === e.id} onClick={() => void install(e, have.theme.id)}>{t('theme-catalog-update', { version: e.version })}</button>}
                </div>
                {shown === e.id && p && (p.ok ? <CatalogPreview theme={p.theme} scheme={hasScheme(p.theme, scheme) ? scheme : p.theme.scheme} />
                  : <ul role="alert" className="m-0 grid list-none gap-1 rounded-s border border-danger p-2 text-xs">{p.lines.map((l, i) => <li key={i}>{l}</li>)}</ul>)}
                {problems?.id === e.id && <ul role="alert" className="m-0 grid list-none gap-1 rounded-s border border-danger p-2 text-xs">{problems.lines.map((l, i) => <li key={i}>{l}</li>)}</ul>}
              </li>
            );
          })}
        </ul>}
    </section>
  );
}

// A theme of the catalog, drawn in its own colours before it is installed: its panels, text, accent, a working mark and a terminal.
// Every colour was checked by the theme rules (#rrggbb only) and is set as a style value, never as CSS text.
function CatalogPreview({ theme, scheme }: { theme: ThemeData; scheme: Scheme }) {
  const look = looksOf(theme)[scheme] as Look;
  const c = look.tokens;
  return (
    <div className="grid overflow-hidden rounded-m border text-xs" style={{ background: c['surface-0'], color: c.text, borderColor: c.border }} aria-hidden data-catalog-preview>
      <div className="grid grid-cols-[6.5rem_minmax(0,1fr)]">
        <div className="grid content-start gap-1 border-e p-2" style={{ background: c['surface-1'], borderColor: c.border }}>
          <span className="font-semibold">{t('theme-region-sidebar')}</span>
          <span><span style={{ color: c['state-working'] }}>●</span> claude</span>
          <span style={{ color: c['text-muted'] }}>codex</span>
        </div>
        <div className="grid min-w-0 content-start gap-2 p-2">
          <div className="flex flex-wrap items-center gap-2">
            <span className="rounded-s px-2 py-1" style={{ background: c.accent, color: c['on-accent'] }}>{t('theme-preview-button')}</span>
            <span className="rounded-s border px-2 py-1" style={{ background: c['surface-2'], borderColor: c.border }}>{t('theme-preview-card')}</span>
          </div>
          <TerminalSample look={look} scheme={scheme} />
        </div>
      </div>
    </div>
  );
}
