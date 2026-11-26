// The theme editor (ADR-0068 rung 2, ADR-0113): a panel of the window, so every change shows on the window itself as it is made. Built-in
// themes are read-only; "Customize" makes a copy the person can change. It edits the colours of the scheme on screen (a theme may have
// both), the colours of the window's parts, the terminal and code, the text and shape, and the window's chrome; what no theme can change
// is listed as locked rows with the reason. A change that would make text hard to read is shown and not applied, so the window always
// stays readable. Text or a control that would be hard to see only in places is listed too, but the change is applied (#1021, #1054).
import { type ReactNode, useId, useRef, useState, useSyncExternalStore } from 'react';
import { t } from '../i18n';
import { ThemePicker } from '../components/ThemePicker';
import { Icon } from '../components/Icon';
import { SegmentedControl } from '../components/kit/SegmentedControl';
import { Toggle } from '../components/kit/Toggle';
import { FONT_TOKENS, MAX_FILE_BYTES, SHAPE_TOKENS, readTheme, regionColour, serialise, terminalColour, withScheme, writeTheme } from './file';
import { type Converted, convertTheme } from './import/convert';
import { ImportDialog } from './ImportDialog';
import { TerminalSample } from './TerminalSample';
import {
  ANSI_NAMES, CHROME, CHROME_NAMES, FLOOR_ITEMS, LINE_HEIGHT, MIN_BASE_TEXT, REGIONS, REGION_PARTS, SYNTAX_KEYS, WEIGHTS,
  type Chrome, type ChromeName, type RegionName, type RegionPart, type SyntaxKey, type TerminalKey, type TextStyle, type Weight,
} from './look';
import {
  chromeLabel, chromeValueLabel, floorLabel, floorReason, fullText, issueText, regionLabel, regionPartLabel, syntaxLabel, terminalLabel,
  themeName, tokenLabel,
} from './messages';
import { type Saved, themeStore } from './store';
import { ThemeCatalog } from './ThemeCatalog';
import { SyncedItems } from './SyncPanels';
import { ExportPack, ImportPack } from './PackDialogs';
import { isPack } from './pack';
import {
  COLOR_TOKENS, type Look, MAX_NAME, type Scheme, type Theme, type ThemeIssue, type TokenName, chromeOf, hasScheme, kindOf, looksOf, pxOf,
  readValue, textOf,
} from './tokens';

export function ThemeEditor() {
  const state = useSyncExternalStore(themeStore.subscribe, themeStore.state, themeStore.state);
  const theme = themeStore.active();
  const scheme = themeStore.scheme();
  const [problems, setProblems] = useState<string[]>([]);
  const file = useRef<HTMLInputElement>(null);
  const other = useRef<HTMLInputElement>(null);
  // A colour file of another app, converted and waiting in the import dialog for Add or Cancel.
  const [imported, setImported] = useState<Converted>();
  const [pack, setPack] = useState<'export' | 'import' | undefined>();
  // An imported theme for the scheme not on screen has no Readability shown, so its warnings are said here; one on screen lists
  // them in its own Readability section.
  const report = (r: Saved) => setProblems(r.ok
    ? (r.warnings && r.theme.id !== themeStore.active().id ? [t('theme-import-warning'), ...r.warnings.map(issueText)] : [])
    : 'full' in r ? [fullText()] : r.issues.map(issueText));
  const themes = themeStore.themes();
  const choice = (s: Scheme) => (
    <label className="grid min-w-0 gap-1">
      <span className="text-xs text-muted">{t(`theme-for-${s}`)}</span>
      <select value={state.choice[s]} onChange={(e) => { themeStore.choose(e.target.value); setProblems([]); }}>
        {themeGroups(themes.filter((x) => hasScheme(x, s))).map((g) => (
          <optgroup key={g.key} label={t(g.key)}>{g.themes.map((x) => <option key={x.id} value={x.id}>{themeName(x)}</option>)}</optgroup>
        ))}
      </select>
    </label>
  );
  const shown: Scheme = hasScheme(theme, scheme) ? scheme : theme.scheme;

  return (
    <section className="@container grid content-start gap-4 p-3" aria-label={t('theme-editor-title')}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 className="m-0">{t('theme-editor-title')}</h2>
        <ThemePicker />
      </div>
      <p className="muted m-0">{t(state.mode === 'system' ? 'theme-follows-system' : 'theme-forced', { scheme: t(`theme-${scheme}`) })}</p>
      <div className="grid grid-cols-2 gap-3">{choice('light')}{choice('dark')}</div>

      <ThemeFields key={`${theme.id}:${shown}`} theme={theme} scheme={shown} onSaved={report} />

      {problems.length > 0 && <ul role="alert" className="m-0 grid list-none gap-1 rounded-s border border-danger p-3 text-xs">
        {problems.map((p, i) => <li key={i}>{p}</li>)}
      </ul>}

      <div className="flex flex-wrap gap-2">
        <button type="button" className="button" onClick={() => file.current?.click()}>{t('theme-import')}</button>
        <button type="button" className="button" onClick={() => other.current?.click()}>{t('theme-import-other')}</button>
        <button type="button" className="button" onClick={() => download(serialise(writeTheme(theme, themeName(theme))))}>{t('theme-export')}</button>
        <button type="button" className="button" onClick={() => setPack('import')}>{t('pack-import')}</button>
        <button type="button" className="button" onClick={() => setPack('export')}>{t('pack-export')}</button>
        {!theme.builtIn && <button type="button" className="button" onClick={() => { themeStore.remove(theme.id); setProblems([]); }}>{t('theme-delete')}</button>}
        <button type="button" className="button" onClick={() => { themeStore.reset(); setProblems([]); }}>{t('theme-reset')}</button>
        <input ref={file} type="file" accept=".json,application/json" className="hidden" aria-hidden tabIndex={-1}
          onChange={async (e) => {
            const f = e.target.files?.[0];
            e.target.value = '';
            // A file over the cap is refused before it is read at all.
            if (!f) return;
            if (f.size > MAX_FILE_BYTES) { report({ ok: false, issues: [{ code: 'too-large', max: MAX_FILE_BYTES / 1024 }] }); return; }
            const text = await f.text();
            // A pack is not a theme: say where it goes rather than "not a theme file".
            if (isPack(parsed(text))) { setProblems([t('theme-import-is-pack')]); return; }
            report(themeStore.import(text));
          }} />
        {/* No `accept`: a Ghostty theme file has no extension. Any file is size-capped, then read as text and never run. */}
        <input ref={other} type="file" className="hidden" aria-hidden tabIndex={-1} data-import-other
          onChange={async (e) => {
            const f = e.target.files?.[0];
            e.target.value = '';
            if (!f) return;
            setProblems([]);
            // A file over the cap is refused before it is read at all.
            setImported(f.size > MAX_FILE_BYTES ? { ok: false, error: 'too-large' } : convertTheme(await f.text(), f.name, t('theme-import-default-name')));
          }} />
      </div>
      {imported && <ImportDialog result={imported} onClose={() => setImported(undefined)} onAdded={(r) => { setImported(undefined); report(r); }} />}
      <p className="muted m-0 text-xs">{t('theme-import-hint')}</p>
      {pack === 'export' && <ExportPack onClose={() => setPack(undefined)} />}
      {pack === 'import' && <ImportPack onClose={() => setPack(undefined)} />}
      <SyncedItems />
      <ThemeCatalog />
    </section>
  );
}

// The theme choices, grouped as the picker shows them: built-in, starter (starter.ts) and the person's own; a group with none is left out.
export function themeGroups(themes: readonly Theme[]): { key: string; themes: Theme[] }[] {
  return [
    { key: 'theme-group-built-in', themes: themes.filter((x) => x.builtIn && !x.starter) },
    { key: 'theme-group-starter', themes: themes.filter((x) => x.starter) },
    { key: 'theme-group-yours', themes: themes.filter((x) => !x.builtIn) },
  ].filter((g) => g.themes.length > 0);
}

// What the fields hold while they are edited: text as typed (a colour left empty follows the theme), lists as chosen.
interface Draft {
  name: string;
  tokens: Record<TokenName, string>;
  regions: Record<string, string>; // `${region}.${part}`
  terminal: Record<string, string>;
  syntax: Record<string, string>;
  chrome: Chrome;
  text: TextStyle;
}
const regionKey = (r: RegionName, p: RegionPart) => `${r}.${p}`;
const HEX = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i;
const optionalOk = (v: string) => v.trim() === '' || HEX.test(v.trim());
const TERMINAL_ROWS: readonly TerminalKey[] = ['background', 'foreground', 'cursor', 'selection'];
const SHAPE_ROWS: readonly TokenName[] = [...FONT_TOKENS, ...SHAPE_TOKENS];
const LINE_HEIGHTS = [1.3, 1.4, 1.5, 1.6, 1.7].filter((h) => h >= LINE_HEIGHT.min && h <= LINE_HEIGHT.max);

export function draftOf(theme: Theme, look: Look): Draft {
  return {
    name: theme.name,
    tokens: { ...look.tokens },
    regions: Object.fromEntries(REGIONS.flatMap((r) => REGION_PARTS.map((p) => [regionKey(r, p), look.regions[r]?.[p] ?? '']))),
    terminal: Object.fromEntries([...TERMINAL_ROWS, ...ANSI_NAMES].map((k) => [k, look.terminal[k] ?? ''])),
    syntax: Object.fromEntries(SYNTAX_KEYS.map((k) => [k, look.syntax[k] ?? ''])),
    chrome: { ...chromeOf(theme) },
    text: { ...textOf(theme) },
  };
}
const filled = (o: Record<string, string>) => Object.fromEntries(Object.entries(o).filter(([, v]) => v.trim() !== '').map(([k, v]) => [k, v.trim()]));
// The theme's file with the scheme on screen as drafted and the other scheme as it is. `complete` is false while a field holds a value
// that is not valid yet (the window keeps the last good one).
export function fileOf(theme: Theme, scheme: Scheme, d: Draft): { file: Record<string, unknown>; complete: boolean } {
  const regions: Record<string, Record<string, string>> = {};
  for (const r of REGIONS) {
    const parts = filled(Object.fromEntries(REGION_PARTS.map((p) => [p, d.regions[regionKey(r, p)] ?? ''])));
    if (Object.keys(parts).length > 0) regions[r] = parts;
  }
  const section: Record<string, unknown> = { tokens: Object.fromEntries(COLOR_TOKENS.map((n) => [n, d.tokens[n]])) };
  if (Object.keys(regions).length > 0) section.regions = regions;
  const terminal = filled(d.terminal);
  if (Object.keys(terminal).length > 0) section.terminal = terminal;
  const syntax = filled(d.syntax);
  if (Object.keys(syntax).length > 0) section.syntax = syntax;
  const file = {
    ...writeTheme(theme, d.name), [scheme]: section,
    shape: Object.fromEntries(SHAPE_TOKENS.map((n) => [n, d.tokens[n]])),
    text: { 'font-ui': d.tokens['font-ui'], 'font-mono': d.tokens['font-mono'], weight: d.text.weight, 'line-height': d.text['line-height'], ligatures: d.text.ligatures },
    chrome: d.chrome,
  };
  const complete = d.name.trim().length > 0
    && [...COLOR_TOKENS, ...SHAPE_ROWS].every((n) => readValue(n, d.tokens[n]) !== undefined)
    && [...Object.values(d.regions), ...Object.values(d.terminal), ...Object.values(d.syntax)].every(optionalOk);
  return { file, complete };
}
const readabilityRefusal = (i: ThemeIssue) => i.code === 'contrast' || i.code === 'region-contrast' || i.code === 'terminal-contrast';

// The fields of the theme on screen. Typed values are kept as typed until they are valid, then saved and shown at once.
function ThemeFields({ theme, scheme, onSaved }: { theme: Theme; scheme: Scheme; onSaved: (r: Saved) => void }) {
  const look = looksOf(theme)[scheme] as Look;
  const [draft, setDraft] = useState<Draft>(() => draftOf(theme, look));
  const ro = !!theme.builtIn;
  const { file, complete } = fileOf(theme, scheme, draft);
  const judged = complete && !ro ? readTheme(file) : undefined;
  const refusals = judged && !judged.ok ? judged.issues : [];
  const warnings = judged?.ok ? judged.warnings ?? [] : [];
  const unreadable = refusals.filter(readabilityRefusal);
  const other = refusals.filter((i) => !readabilityRefusal(i));
  const otherScheme: Scheme = scheme === 'dark' ? 'light' : 'dark';

  const change = (next: Draft) => {
    setDraft(next);
    const f = fileOf(theme, scheme, next);
    if (f.complete) themeStore.update(theme.id, { file: f.file });
  };
  const setToken = (n: TokenName, v: string) => change({ ...draft, tokens: { ...draft.tokens, [n]: v } });
  const setRegion = (r: RegionName, p: RegionPart, v: string) => change({ ...draft, regions: { ...draft.regions, [regionKey(r, p)]: v } });
  const setTerminal = (k: TerminalKey, v: string) => change({ ...draft, terminal: { ...draft.terminal, [k]: v } });
  const setSyntax = (k: SyntaxKey, v: string) => change({ ...draft, syntax: { ...draft.syntax, [k]: v } });
  const setChrome = (n: ChromeName, v: string | number) => change({ ...draft, chrome: { ...draft.chrome, [n]: v } });
  const setText = <K extends keyof TextStyle>(n: K, v: TextStyle[K]) => change({ ...draft, text: { ...draft.text, [n]: v } });
  const addScheme = () => onSaved(themeStore.update(theme.id, { file: writeTheme({ ...theme, looks: withScheme(looksOf(theme), otherScheme) }, theme.name) }));

  return (
    <div className="grid gap-4">
      <div className="flex flex-wrap items-end gap-2">
        {ro ? <p className="m-0 min-w-0 flex-1 text-sm"><span className="font-semibold">{themeName(theme)}</span> <span className="muted whitespace-nowrap">· {t(theme.starter ? 'theme-starter' : 'theme-built-in')}</span></p>
          : <label className="grid min-w-0 flex-1 gap-1"><span className="text-xs text-muted">{t('theme-name')}</span>
            <input value={draft.name} maxLength={MAX_NAME} onChange={(e) => change({ ...draft, name: e.target.value })} aria-invalid={draft.name.trim().length === 0} /></label>}
        {ro && <button type="button" className="button button--primary" onClick={() => onSaved(themeStore.copy(theme, t('theme-copy-name', { name: themeName(theme) })))}>{t('theme-customize')}</button>}
      </div>
      {ro && <p className="muted m-0 text-xs">{theme.starter ? t('theme-starter-hint', { ...theme.starter }) : t('theme-built-in-hint')}</p>}

      <Preview look={look} scheme={scheme} />

      <fieldset className="m-0 grid gap-1 border-0 p-0" disabled={ro}>
        <legend className="ide-section px-0">{t('theme-colours')}</legend>
        {hasScheme(theme, otherScheme) ? <p className="muted m-0 mb-1 text-xs">{t('theme-scheme-editing', { scheme: t(`theme-${scheme}`) })}</p>
          : !ro && <div className="mb-1 flex flex-wrap items-center gap-2"><span className="muted min-w-0 flex-1 text-xs">{t('theme-scheme-add-hint', { scheme: t(`theme-${scheme}`) })}</span>
            <button type="button" className="kit-btn kit-btn--sm" onClick={addScheme}>{t('theme-scheme-add', { scheme: t(`theme-${otherScheme}`) })}</button></div>}
        {COLOR_TOKENS.map((n) => <ColorRow key={n} label={tokenLabel(n)} value={draft.tokens[n]} valid={readValue(n, draft.tokens[n]) !== undefined} onChange={(v) => setToken(n, v)} />)}
      </fieldset>

      <Group title={t('theme-regions')} hint={t('theme-regions-hint')} disabled={ro}>
        {REGIONS.map((r) => (
          <div key={r} className="grid gap-1" role="group" aria-label={regionLabel(r)} data-region-rows={r}>
            <span className="text-xs font-semibold">{regionLabel(r)}</span>
            {REGION_PARTS.map((p) => <ColorRow key={p} label={regionPartLabel(r, p)} short={t(`theme-part-${p}`)} value={draft.regions[regionKey(r, p)] ?? ''} optional
              shown={regionColour(look, scheme, r, p)} valid={optionalOk(draft.regions[regionKey(r, p)] ?? '')} onChange={(v) => setRegion(r, p, v)} />)}
          </div>
        ))}
      </Group>

      <Group title={t('theme-terminal')} hint={t('theme-terminal-hint')} disabled={ro}>
        {[...TERMINAL_ROWS, ...ANSI_NAMES].map((k) => <ColorRow key={k} label={terminalLabel(k)} value={draft.terminal[k] ?? ''} optional shown={terminalColour(look, scheme, k)}
          valid={optionalOk(draft.terminal[k] ?? '')} onChange={(v) => setTerminal(k, v)} />)}
      </Group>

      <Group title={t('theme-syntax')} hint={t('theme-syntax-hint')} disabled={ro}>
        {SYNTAX_KEYS.map((k) => <ColorRow key={k} label={syntaxLabel(k)} value={draft.syntax[k] ?? ''} optional valid={optionalOk(draft.syntax[k] ?? '')} onChange={(v) => setSyntax(k, v)}
          swatch={`var(--syntax-${k})`} />)}
      </Group>

      <fieldset className="m-0 grid gap-2 border-0 p-0" disabled={ro}>
        <legend className="ide-section px-0">{t('theme-shape')}</legend>
        {SHAPE_ROWS.map((n) => <ShapeRow key={n} name={n} value={draft.tokens[n]} valid={readValue(n, draft.tokens[n]) !== undefined} onChange={(v) => setToken(n, v)} />)}
        <Row label={t('theme-text-weight')}>
          <SegmentedControl label={t('theme-text-weight')} disabled={ro} value={String(draft.text.weight)} onChange={(v) => setText('weight', Number(v) as Weight)}
            items={WEIGHTS.map((w) => ({ value: String(w), label: t(`theme-text-weight-${w}`) }))} />
        </Row>
        <Row label={t('theme-text-line-height')}>
          <select className="py-1 text-xs" value={String(draft.text['line-height'])} disabled={ro} aria-label={t('theme-text-line-height')}
            onChange={(e) => setText('line-height', e.target.value === 'normal' ? 'normal' : Number(e.target.value))}>
            <option value="normal">{t('theme-text-line-height-normal')}</option>
            {LINE_HEIGHTS.map((h) => <option key={h} value={String(h)}>{h}</option>)}
          </select>
        </Row>
        <Row label={t('theme-text-ligatures')} hint={t('theme-text-ligatures-hint')}>
          <Toggle label={t('theme-text-ligatures')} checked={draft.text.ligatures} disabled={ro} onChange={(v) => setText('ligatures', v)} />
        </Row>
      </fieldset>

      <fieldset className="m-0 grid gap-2 border-0 p-0" disabled={ro}>
        <legend className="ide-section px-0">{t('theme-chrome')}</legend>
        {CHROME_NAMES.map((n) => <ChromeRow key={n} name={n} value={draft.chrome[n]} disabled={ro} onChange={(v) => setChrome(n, v)} />)}
      </fieldset>

      <Floor />

      <div className="grid gap-1" aria-live="polite">
        <h3 className="ide-section m-0 px-0">{t('theme-readability')}</h3>
        {ro ? <p className="state state--finished m-0">{t('theme-contrast-ok')}</p>
          : !complete ? <p className="m-0 text-xs text-danger">{t('theme-incomplete')}</p>
          : refusals.length === 0 && warnings.length === 0 ? <p className="state state--finished m-0">{t('theme-contrast-ok')}</p>
          : <>
            {unreadable.length > 0 && <ul className="m-0 grid list-none gap-1 p-0 text-xs">
              <li className="text-danger">{t('theme-contrast-not-applied')}</li>
              {unreadable.map((c) => <li key={JSON.stringify(c)}>{issueText(c)}</li>)}
            </ul>}
            {other.map((i) => <p key={JSON.stringify(i)} className="m-0 text-xs text-danger">{issueText(i)}</p>)}
            {warnings.length > 0 && <ul className="m-0 grid list-none gap-1 p-0 text-xs">
              <li className="text-danger">{t(refusals.length > 0 ? 'theme-contrast-also' : 'theme-contrast-warning')}</li>
              {warnings.map((w) => <li key={JSON.stringify(w)}>{issueText(w)}</li>)}
            </ul>}
          </>}
      </div>
    </div>
  );
}

// What no theme can change: locked rows with the reason (ADR-0113 decision 3).
export function Floor() {
  const id = useId();
  return (
    <div className="grid gap-1" role="group" aria-labelledby={id}>
      <h3 id={id} className="ide-section m-0 px-0">{t('theme-floor')}</h3>
      <p className="muted m-0 text-xs">{t('theme-floor-hint')}</p>
      <ul className="m-0 grid list-none gap-1 p-0">
        {FLOOR_ITEMS.map((f) => (
          <li key={f} className="grid grid-cols-[auto_minmax(0,1fr)] items-start gap-x-2 rounded-xs bg-surface-2 px-2 py-1.5 text-xs" data-floor={f}>
            <Icon name="lock" className="mt-0.5 size-3.5 text-muted" />
            <span><span className="font-semibold">{floorLabel(f)}</span><span className="sr-only"> · {t('theme-locked')}</span><span className="block text-muted">{floorReason(f)}</span></span>
          </li>
        ))}
      </ul>
    </div>
  );
}

function Group({ title, hint, disabled, children }: { title: string; hint: string; disabled: boolean; children: ReactNode }) {
  return (
    <fieldset className="m-0 grid gap-2 border-0 p-0" disabled={disabled}>
      <legend className="ide-section px-0">{title}</legend>
      <p className="muted m-0 text-xs">{hint}</p>
      {children}
    </fieldset>
  );
}

function Row({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1">
      <span className="grid min-w-0 text-xs">{label}{hint && <span className="text-muted">{hint}</span>}</span>
      {children}
    </div>
  );
}

// A colour: a swatch and a hex field. An optional one may be left empty: its field then shows the colour it follows.
function ColorRow({ label, short, value, valid, optional, shown, swatch, onChange }: {
  label: string; short?: string; value: string; valid: boolean; optional?: boolean; shown?: string; swatch?: string; onChange: (v: string) => void;
}) {
  const id = useId();
  const own = value.trim() !== '' && valid ? readValue('accent', value) : undefined;
  const picker = own ?? (shown && HEX.test(shown) ? shown : '#000000');
  return (
    <div className="grid grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-x-2 gap-y-1 @xs:grid-cols-[minmax(0,1fr)_auto_7rem_auto]">
      <label htmlFor={id} className="col-span-3 truncate text-xs @xs:col-span-1" title={label}>{short ?? label}</label>
      {optional && swatch && !own ? <span aria-hidden className="h-7 w-9 rounded-xs border border-line" style={{ background: swatch }} />
        : <input type="color" className="h-7 w-9 cursor-pointer p-0.5" value={picker} onChange={(e) => onChange(e.target.value)} aria-label={label} tabIndex={-1} />}
      <input id={id} className="px-2 py-1 font-mono text-xs" value={value} spellCheck={false} aria-invalid={!valid} aria-label={short ? label : undefined}
        placeholder={optional ? (shown && HEX.test(shown) ? shown : t('theme-follows')) : undefined} onChange={(e) => onChange(e.target.value)} />
      {optional ? <button type="button" className="ide-iconbtn size-6" disabled={value === ''} onClick={() => onChange('')} aria-label={`${t('theme-clear')}: ${label}`} title={t('theme-clear')}>
        <Icon name="close" className="size-3" /></button> : <span />}
    </div>
  );
}

function ShapeRow({ name, value, valid, onChange }: { name: TokenName; value: string; valid: boolean; onChange: (v: string) => void }) {
  const id = useId();
  const k = kindOf(name);
  if (k.kind === 'font') {
    return <label className="grid gap-1"><span className="text-xs">{tokenLabel(name)}</span>
      <input className="px-2 py-1 text-xs" value={value} spellCheck={false} aria-invalid={!valid} onChange={(e) => onChange(e.target.value)} /></label>;
  }
  if (k.kind !== 'px') return null;
  // No text under 12 px: the base size starts at 16 (ADR-0113).
  const min = name === 'text-16' ? MIN_BASE_TEXT : k.min;
  const n = valid ? Math.max(min, pxOf(value)) : min;
  return (
    <div className="grid grid-cols-[minmax(0,1fr)_3rem] items-center gap-x-2 gap-y-1 @xs:grid-cols-[minmax(0,1fr)_minmax(0,8rem)_3rem]">
      <label htmlFor={id} className="col-span-2 truncate text-xs @xs:col-span-1">{tokenLabel(name)}</label>
      <input id={id} type="range" className="border-0 bg-transparent p-0 accent-accent" min={min} max={k.max} step={1} value={n} onChange={(e) => onChange(`${e.target.value}px`)} />
      <span className="text-end font-mono text-xs text-muted">{t('theme-px', { size: n })}</span>
    </div>
  );
}

function ChromeRow({ name, value, disabled, onChange }: { name: ChromeName; value: string | number; disabled: boolean; onChange: (v: string | number) => void }) {
  const id = useId();
  const kind = CHROME[name];
  const hint = name === 'state-display' || name === 'icons' ? t(`theme-chrome-${name}-hint`) : undefined;
  if (kind.kind === 'px') {
    return (
      <div className="grid grid-cols-[minmax(0,1fr)_3rem] items-center gap-x-2 gap-y-1 @xs:grid-cols-[minmax(0,1fr)_minmax(0,8rem)_3rem]">
        <label htmlFor={id} className="col-span-2 truncate text-xs @xs:col-span-1">{chromeLabel(name)}</label>
        <input id={id} type="range" className="border-0 bg-transparent p-0 accent-accent" min={kind.min} max={kind.max} step={1} value={Number(value)} disabled={disabled}
          onChange={(e) => onChange(Number(e.target.value))} />
        <span className="text-end font-mono text-xs text-muted">{t('theme-px', { size: Number(value) })}</span>
      </div>
    );
  }
  return (
    <Row label={chromeLabel(name)} hint={hint}>
      <SegmentedControl label={chromeLabel(name)} disabled={disabled} value={String(value)} onChange={(v) => onChange(v)}
        items={(kind.values as readonly string[]).map((v) => ({ value: v, label: chromeValueLabel(name, v) }))} />
    </Row>
  );
}

// A small copy of each part of the window, drawn by the same rules as the window (the theme on screen), so a change shows here too.
function Preview({ look, scheme }: { look: Look; scheme: Scheme }) {
  return (
    <div className="grid gap-2">
      <h3 className="ide-section m-0 px-0">{t('theme-preview')}</h3>
      <div className="grid overflow-hidden rounded-m border border-line bg-surface-0 text-xs" aria-hidden data-theme-preview>
        <div className="grid grid-cols-[6.5rem_minmax(0,1fr)]">
          <div className="grid content-start gap-1 border-e border-sidebar-line bg-sidebar p-2 text-sidebar-ink" data-preview="sidebar">
            <span className="font-semibold">{t('theme-region-sidebar')}</span>
            <span className="state state--working"><svg className="state__mark" viewBox="0 0 12 12"><circle cx="6" cy="6" r="5" fill="currentColor" /></svg><span className="sr-only">{t('state-working')}</span>claude</span>
            <span className="text-muted">codex</span>
          </div>
          <div className="grid min-w-0 content-start">
            <div className="flex h-8 items-stretch overflow-hidden border-b border-panebar-line bg-panebar text-panebar-ink" data-preview="pane-bar">
              <span className="ide-tab min-w-0" aria-selected="true" style={{ height: 'auto' }}>{t('theme-preview-tab')}</span>
              <span className="ide-tab min-w-0" style={{ height: 'auto' }}>{t('theme-preview-pane')}</span>
            </div>
            <div className="grid gap-2 p-2">
              <div className="chat-agent" data-preview="agent-entries">{t('theme-preview-agent')}</div>
              <div className="ms-auto rounded-l rounded-ee-xs border border-personentry-line bg-personentry px-2.5 py-1.5 text-personentry-ink" data-preview="person-entries">{t('theme-preview-person')}</div>
              <div className="overflow-hidden rounded-s border border-codeblock-line bg-codeblock font-mono text-codeblock-ink" data-preview="code-blocks">
                <pre className="m-0 p-2 text-xs"><span className="hljs-keyword">const</span> n = <span className="hljs-number">42</span>; <span className="hljs-comment">{'// ok'}</span></pre>
              </div>
              <div className="flex flex-wrap items-center gap-2">
                <span className="kit-card kit-card--dense" data-preview="cards">{t('theme-preview-card')}</span>
                <span className="kit-btn kit-btn--sm" data-preview="buttons">{t('theme-preview-button')}</span>
                <span className="rounded-s border border-input-line bg-input px-2 py-1 text-input-ink" data-preview="inputs">{t('theme-preview-field')}</span>
              </div>
              <TerminalSample look={look} scheme={scheme} />
            </div>
          </div>
        </div>
        <div className="flex h-6 items-center gap-2 border-t border-statusline-line bg-statusline px-2 text-statusline-ink" data-preview="status-line">{t('theme-preview-status')}</div>
      </div>
    </div>
  );
}

function parsed(text: string): unknown {
  try { return JSON.parse(text); } catch { return undefined; }
}

function download(text: string) {
  try {
    const url = URL.createObjectURL(new Blob([text], { type: 'application/json' }));
    const a = document.createElement('a');
    a.href = url;
    a.download = 'theme.json';
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  } catch { /* nothing to save to here */ }
}
