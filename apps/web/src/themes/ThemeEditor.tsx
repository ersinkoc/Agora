// The theme editor (ADR-0068 rung 2): a panel of the window, so every change shows on the window itself as it is made. Built-in
// themes are read-only; "Customize" makes a copy the person can change. A change that would make text hard to read is shown as
// a warning and not applied, so the window always stays readable.
import { useId, useRef, useState, useSyncExternalStore } from 'react';
import { t } from '../i18n';
import { ThemePicker } from '../components/ThemePicker';
import { fullText, issueText, themeName, tokenLabel } from './messages';
import { type Saved, themeStore } from './store';
import {
  COLOR_TOKENS, TOKEN_NAMES, type Scheme, type Theme, type ThemeIssue, type TokenName, type Tokens,
  MAX_NAME, contrastIssues, kindOf, pxOf, readValue, serialiseTheme, themeFile,
} from './tokens';

const SHAPE_TOKENS = TOKEN_NAMES.filter((n) => kindOf(n).kind !== 'color');

export function ThemeEditor() {
  const state = useSyncExternalStore(themeStore.subscribe, themeStore.state, themeStore.state);
  const theme = themeStore.active();
  const [problems, setProblems] = useState<string[]>([]);
  const file = useRef<HTMLInputElement>(null);
  const report = (r: Saved) => setProblems(r.ok ? [] : 'full' in r ? [fullText()] : r.issues.map(issueText));
  const themes = themeStore.themes();
  const choice = (scheme: Scheme) => (
    <label className="grid min-w-0 gap-1">
      <span className="text-xs text-muted">{t(`theme-for-${scheme}`)}</span>
      <select value={state.choice[scheme]} onChange={(e) => { themeStore.choose(e.target.value); setProblems([]); }}>
        {themes.filter((x) => x.scheme === scheme).map((x) => <option key={x.id} value={x.id}>{themeName(x)}</option>)}
      </select>
    </label>
  );

  return (
    <section className="@container grid content-start gap-4 p-3" aria-label={t('theme-editor-title')}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 className="m-0">{t('theme-editor-title')}</h2>
        <ThemePicker />
      </div>
      <p className="muted m-0">{t(state.mode === 'system' ? 'theme-follows-system' : 'theme-forced', { scheme: t(`theme-${themeStore.scheme()}`) })}</p>
      <div className="grid grid-cols-2 gap-3">{choice('light')}{choice('dark')}</div>

      <ThemeFields key={theme.id} theme={theme} onSaved={report} />

      {problems.length > 0 && <ul role="alert" className="m-0 grid list-none gap-1 rounded-s border border-needs-you p-3 text-xs">
        {problems.map((p, i) => <li key={i}>{p}</li>)}
      </ul>}

      <div className="flex flex-wrap gap-2">
        <button type="button" className="button" onClick={() => file.current?.click()}>{t('theme-import')}</button>
        <button type="button" className="button" onClick={() => download(serialiseTheme(themeFile(theme, themeName(theme))))}>{t('theme-export')}</button>
        {!theme.builtIn && <button type="button" className="button" onClick={() => { themeStore.remove(theme.id); setProblems([]); }}>{t('theme-delete')}</button>}
        <button type="button" className="button" onClick={() => { themeStore.reset(); setProblems([]); }}>{t('theme-reset')}</button>
        <input ref={file} type="file" accept=".json,application/json" className="hidden" aria-hidden tabIndex={-1}
          onChange={async (e) => { const f = e.target.files?.[0]; e.target.value = ''; if (f) report(themeStore.import(await f.text())); }} />
      </div>
      <p className="muted m-0 text-xs">{t('theme-import-hint')}</p>
    </section>
  );
}

// The tokens of the theme on screen. Typed values are kept as typed until they are valid, then saved and shown at once.
function ThemeFields({ theme, onSaved }: { theme: Theme; onSaved: (r: Saved) => void }) {
  const [draft, setDraft] = useState<Record<TokenName, string>>(() => ({ ...theme.tokens }));
  const [name, setName] = useState(theme.name);
  const [issues, setIssues] = useState<ThemeIssue[]>([]);
  const ro = !!theme.builtIn;
  const parsed = Object.fromEntries(TOKEN_NAMES.map((n) => [n, readValue(n, draft[n])])) as Record<TokenName, string | undefined>;
  const complete = TOKEN_NAMES.every((n) => parsed[n] !== undefined);
  const contrast = complete ? contrastIssues(parsed as Tokens) : [];

  const change = (next: Record<TokenName, string>, nextName = name) => {
    setDraft(next);
    setName(nextName);
    const tokens = Object.fromEntries(TOKEN_NAMES.map((n) => [n, readValue(n, next[n])]));
    if (Object.values(tokens).some((v) => v === undefined) || nextName.trim().length === 0) return; // still typing
    const r = themeStore.update(theme.id, { name: nextName, tokens: tokens as Tokens });
    setIssues(r.ok || 'full' in r ? [] : r.issues.filter((i) => i.code !== 'contrast'));
  };
  const set = (n: TokenName, v: string) => change({ ...draft, [n]: v });

  return (
    <div className="grid gap-4">
      <div className="flex flex-wrap items-end gap-2">
        {ro ? <p className="m-0 min-w-0 flex-1 text-sm"><span className="font-semibold">{themeName(theme)}</span> <span className="muted whitespace-nowrap">· {t('theme-built-in')}</span></p>
          : <label className="grid min-w-0 flex-1 gap-1"><span className="text-xs text-muted">{t('theme-name')}</span>
            <input value={name} maxLength={MAX_NAME} onChange={(e) => change(draft, e.target.value)} aria-invalid={name.trim().length === 0} /></label>}
        {ro && <button type="button" className="button button--primary" onClick={() => onSaved(themeStore.copy(theme, t('theme-copy-name', { name: themeName(theme) })))}>{t('theme-customize')}</button>}
      </div>
      {ro && <p className="muted m-0 text-xs">{t('theme-built-in-hint')}</p>}

      <fieldset className="m-0 grid gap-1 border-0 p-0" disabled={ro}>
        <legend className="ide-section px-0">{t('theme-colours')}</legend>
        {COLOR_TOKENS.map((n) => <ColorRow key={n} name={n} value={draft[n]} valid={parsed[n] !== undefined} onChange={(v) => set(n, v)} />)}
      </fieldset>

      <fieldset className="m-0 grid gap-2 border-0 p-0" disabled={ro}>
        <legend className="ide-section px-0">{t('theme-shape')}</legend>
        {SHAPE_TOKENS.map((n) => <ShapeRow key={n} name={n} value={draft[n]} valid={parsed[n] !== undefined} onChange={(v) => set(n, v)} />)}
      </fieldset>

      <div className="grid gap-1" aria-live="polite">
        <h3 className="ide-section m-0 px-0">{t('theme-readability')}</h3>
        {!complete ? <p className="m-0 text-xs text-needs-you">{t('theme-incomplete')}</p>
          : contrast.length === 0 ? <p className="state state--finished m-0">{t('theme-contrast-ok')}</p>
          : <ul className="m-0 grid list-none gap-1 p-0 text-xs">
            <li className="text-needs-you">{t('theme-contrast-not-applied')}</li>
            {contrast.map((c) => <li key={`${c.fg}/${c.bg}`}>{issueText({ code: 'contrast', ...c })}</li>)}
          </ul>}
        {issues.map((i, k) => <p key={k} className="m-0 text-xs text-needs-you">{issueText(i)}</p>)}
      </div>
    </div>
  );
}

function ColorRow({ name, value, valid, onChange }: { name: TokenName; value: string; valid: boolean; onChange: (v: string) => void }) {
  const id = useId();
  const hex = readValue(name, value);
  return (
    <div className="grid grid-cols-[auto_minmax(0,1fr)] items-center gap-x-2 gap-y-1 @xs:grid-cols-[minmax(0,1fr)_auto_7rem]">
      <label htmlFor={id} className="col-span-2 truncate text-xs @xs:col-span-1">{tokenLabel(name)}</label>
      <input type="color" className="h-7 w-9 cursor-pointer p-0.5" value={hex ?? '#000000'} onChange={(e) => onChange(e.target.value)} aria-label={tokenLabel(name)} tabIndex={-1} />
      <input id={id} className="px-2 py-1 font-mono text-xs" value={value} spellCheck={false} aria-invalid={!valid} onChange={(e) => onChange(e.target.value)} />
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
  const n = valid ? pxOf(value) : k.min;
  return (
    <div className="grid grid-cols-[minmax(0,1fr)_3rem] items-center gap-x-2 gap-y-1 @xs:grid-cols-[minmax(0,1fr)_minmax(0,8rem)_3rem]">
      <label htmlFor={id} className="col-span-2 truncate text-xs @xs:col-span-1">{tokenLabel(name)}</label>
      <input id={id} type="range" className="border-0 bg-transparent p-0 accent-accent" min={k.min} max={k.max} step={1} value={n} onChange={(e) => onChange(`${e.target.value}px`)} />
      <span className="text-end font-mono text-xs text-muted">{t('theme-px', { size: n })}</span>
    </div>
  );
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
