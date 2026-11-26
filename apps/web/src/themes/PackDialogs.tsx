// Theme packs in the window (ADR-0113 decision 5, docs/ux/themes.md "Theme packs"): "Export as pack" writes the themes, saved layouts and
// keybindings the person picks into one file; "Import pack" reads one, shows what it holds and applies only the parts the person ticks
// (nothing is ticked at first). Themes are added to the person's own (none is chosen for them), layouts are added as saved layouts (never
// replacing one), and keybindings are merged only after a step that shows what they would change. Every text from a pack is shown as text.
import * as Dialog from '@radix-ui/react-dialog';
import { type ReactNode, useMemo, useState, useSyncExternalStore } from 'react';
import { t } from '../i18n';
import { Icon } from '../components/Icon';
import { Button } from '../components/kit';
import { type KeyStore, keyStore } from '../keys/store';
import { mergeKeys } from '../keys/file';
import { commandById } from '../keys/commands';
import { commandLabel } from '../keys/view';
import { issueText as keyIssueText } from '../keys/KeyboardSettings';
import { errorText as layoutErrorText } from '../layout/LayoutMenu';
import { PANEL_INFO } from '../layout/panels';
import { MAX_SAVED, addNamed, savedLayouts } from '../layout/store';
import { panesOf } from '../layout/tree';
import { IS_MAC, bindingLabel, parseBinding } from '../keys/strokes';
import { fullText, issueText, themeName } from './messages';
import {
  MAX_PACK_BYTES, type Pack, type PackIssue, deserialisePack, missingPanels, serialisePack, writePack,
} from './pack';
import { type KeptPack, type PackStore, packStore } from './kept';
import { type ThemeStore, themeStore } from './store';
import { MAX_NAME, schemesOf } from './tokens';

/** A problem of a pack in words: a part's own problem says which theme or layout it is in. */
export function packIssueText(i: PackIssue): string {
  const n = (index: number) => index + 1;
  switch (i.code) {
    case 'too-large': return t('pack-error-too-large', { max: i.max });
    case 'unknown-field': return t('pack-error-unknown-field', { field: i.field });
    case 'name': return t('pack-error-name', { max: MAX_NAME });
    case 'part': return t('pack-error-part', { part: i.part });
    case 'too-many': return t(`pack-error-too-many-${i.part}`, { max: i.max });
    case 'theme-format': return t('pack-error-theme-format', { n: n(i.index) });
    case 'theme': return t('pack-in-theme', { n: n(i.index), text: issueText(i.issue) });
    case 'layout-entry': case 'layout-name': case 'layout-agent': return t(`pack-error-${i.code}`, { n: n(i.index) });
    case 'layout-twice': return t('pack-error-layout-twice', { n: n(i.index), name: i.name });
    case 'layout-field': return t('pack-error-layout-field', { n: n(i.index), field: i.field });
    case 'layout-panel': return t('pack-error-layout-panel', { n: n(i.index), panel: i.panel });
    case 'layout': return t('pack-in-layout', { n: n(i.index), text: layoutErrorText(i.error) });
    case 'keys': return t('pack-in-keys', { text: keyIssueText(i.issue) });
    case 'more': return t('pack-error-more', { count: i.count });
    default: return t(`pack-error-${i.code}`);
  }
}

function download(text: string) {
  try {
    const url = URL.createObjectURL(new Blob([text], { type: 'application/json' }));
    const a = document.createElement('a');
    a.href = url;
    a.download = 'theme-pack.json';
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  } catch { /* nothing to save to here */ }
}

function Shell({ title, icon, children, onClose, label }: { title: string; icon: 'download' | 'upload'; children: ReactNode; onClose: () => void; label: string }) {
  return (
    <Dialog.Root open onOpenChange={(open) => { if (!open) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="kit-overlay" />
        <Dialog.Content aria-describedby={undefined} className="kit-dialog pack-dialog" data-pack-dialog={label}>
          <div className="flex items-center gap-3">
            <span aria-hidden className="kit-chip kit-tone-accent"><Icon name={icon} /></span>
            <Dialog.Title className="kit-dialog__title">{title}</Dialog.Title>
          </div>
          {children}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <fieldset className="m-0 grid gap-1.5 border-0 p-0">
      <legend className="ide-section px-0">{title}</legend>
      {children}
    </fieldset>
  );
}

function Check({ checked, disabled, onChange, title, meta, children }: {
  checked: boolean; disabled?: boolean; onChange: (on: boolean) => void; title: ReactNode; meta?: ReactNode; children?: ReactNode;
}) {
  return (
    <label className="pack-row" data-disabled={disabled || undefined}>
      <input type="checkbox" checked={checked} disabled={disabled} onChange={(e) => onChange(e.target.checked)} />
      <span className="grid min-w-0 flex-1 gap-0.5">
        <span className="truncate font-medium">{title}</span>
        {meta && <span className="text-xs text-muted">{meta}</span>}
        {children}
      </span>
    </label>
  );
}

const schemesText = (s: readonly ('dark' | 'light')[]) => s.map((x) => t(`theme-${x}`)).join(' · ');
const toggled = (set: ReadonlySet<number>, i: number, on: boolean) => { const next = new Set(set); if (on) next.add(i); else next.delete(i); return next; };

// ---- Export -----------------------------------------------------------------------------------------------------------------------

export function ExportPack({ onClose, themes: store = themeStore, keys = keyStore }: { onClose: () => void; themes?: ThemeStore; keys?: KeyStore }) {
  const all = store.themes();
  const ownFirst = [...all.filter((x) => !x.builtIn), ...all.filter((x) => x.builtIn)];
  const active = store.active();
  const layouts = useMemo(() => savedLayouts(), []);
  const keyFile = keys.state().file;
  const [name, setName] = useState(() => (active.builtIn ? t('pack-name-default') : themeName(active)));
  const [themes, setThemes] = useState<ReadonlySet<number>>(() => new Set([ownFirst.findIndex((x) => x.id === active.id)]));
  const [chosenLayouts, setLayouts] = useState<ReadonlySet<number>>(new Set());
  const [withKeys, setWithKeys] = useState(false);
  const [problem, setProblem] = useState<string | undefined>();
  const nothing = themes.size === 0 && chosenLayouts.size === 0 && !withKeys;
  const bound = [...chosenLayouts].some((i) => panesOf(layouts[i]!.layout.root).some((p) => p.bind && 'agent' in p.bind));

  const save = () => {
    const file = writePack({
      name: name.trim(),
      themes: ownFirst.filter((_, i) => themes.has(i)).map((x) => ({ theme: x, name: themeName(x) })),
      layouts: layouts.filter((_, i) => chosenLayouts.has(i)),
      keys: withKeys ? keyFile : undefined,
    });
    const text = serialisePack(file);
    // What is written must read back: a pack over the cap would be refused by every window, this one included.
    const back = deserialisePack(text);
    if (!back.ok) { setProblem(back.issues.map(packIssueText).join(' ')); return; }
    download(text);
    onClose();
  };

  return (
    <Shell title={t('pack-export-title')} icon="download" onClose={onClose} label="export">
      <p className="muted m-0 text-xs">{t('pack-export-hint')}</p>
      <label className="grid gap-1"><span className="text-xs text-muted">{t('pack-name')}</span>
        <input value={name} maxLength={MAX_NAME} onChange={(e) => { setName(e.target.value); setProblem(undefined); }} aria-invalid={name.trim().length === 0} /></label>
      <Section title={t('pack-themes')}>
        <div className="pack-list">
          {ownFirst.map((x, i) => <Check key={x.id} checked={themes.has(i)} onChange={(on) => setThemes((s) => toggled(s, i, on))}
            title={themeName(x)} meta={`${schemesText(schemesOf(x))}${x.builtIn ? ` · ${t('theme-built-in')}` : ''}`} />)}
        </div>
      </Section>
      <Section title={t('pack-layouts')}>
        {layouts.length === 0 ? <p className="muted m-0 text-xs">{t('pack-export-no-layouts')}</p> : <div className="pack-list">
          {layouts.map((l, i) => <Check key={l.name} checked={chosenLayouts.has(i)} onChange={(on) => setLayouts((s) => toggled(s, i, on))}
            title={l.name} meta={t('pack-panes', { count: panesOf(l.layout.root).length })} />)}
        </div>}
        {bound && <p className="muted m-0 text-xs">{t('pack-export-unbound')}</p>}
      </Section>
      <Section title={t('pack-keys')}>
        <Check checked={withKeys} disabled={keyFile.entries.length === 0} onChange={setWithKeys}
          title={t('pack-keys-own')} meta={keyFile.entries.length === 0 ? t('pack-keys-none') : t('pack-keys-count', { count: keyFile.entries.length })} />
      </Section>
      {problem && <p role="alert" className="m-0 text-xs text-danger">{problem}</p>}
      <div className="kit-dialog__actions">
        <Button onClick={onClose}>{t('layout-cancel')}</Button>
        <Button variant="primary" disabled={nothing || name.trim().length === 0} onClick={save}>{t('pack-export-do')}</Button>
      </div>
    </Shell>
  );
}

// ---- Import -----------------------------------------------------------------------------------------------------------------------

type Step = { at: 'read' } | { at: 'preview'; pack: Pack } | { at: 'keys'; pack: Pack } | { at: 'done'; lines: string[] };
const knownPanel = (id: string) => !!PANEL_INFO.get(id);

// `kept`: one of the person's kept packs (from this or another of their devices), opened straight at its preview. A pack imported here
// is kept, so the person's other devices can offer it (ADR-0113 step 4, kept.ts); nothing of it is applied there either until picked.
export function ImportPack({ onClose, themes: store = themeStore, keys = keyStore, packs = packStore, mac = IS_MAC, kept }: {
  onClose: () => void; themes?: ThemeStore; keys?: KeyStore; packs?: PackStore; mac?: boolean; kept?: KeptPack;
}) {
  const [step, setStep] = useState<Step>(kept ? { at: 'preview', pack: kept.pack } : { at: 'read' });
  const [text, setText] = useState(() => (kept ? serialisePack(kept.file) : ''));
  const [issues, setIssues] = useState<PackIssue[]>([]);
  const [themes, setThemes] = useState<ReadonlySet<number>>(new Set());
  const [layouts, setLayouts] = useState<ReadonlySet<number>>(new Set());
  const [withKeys, setWithKeys] = useState(false);
  const [problem, setProblem] = useState<string | undefined>();
  const keyState = useSyncExternalStore(keys.subscribe, keys.state, keys.state);

  const preview = () => {
    const r = deserialisePack(text);
    if (!r.ok) { setIssues(r.issues); return; }
    setThemes(new Set()); setLayouts(new Set()); setWithKeys(false); setProblem(undefined);
    setStep({ at: 'preview', pack: r.pack });
  };
  const merge = (pack: Pack) => (pack.keys ? mergeKeys(keyState.file, keyState.unknown, pack.keys.file, pack.keys.unknown) : undefined);

  // Everything chosen is checked before anything is kept, so a part that cannot be applied leaves every part as it was.
  const apply = (pack: Pack) => {
    const chosenThemes = pack.themes.filter((_, i) => themes.has(i)).map((x) => x.theme);
    const chosenLayouts = pack.layouts.filter((_, i) => layouts.has(i));
    if (chosenThemes.length > store.room()) { setProblem(fullText()); return; }
    if (chosenLayouts.length > 0 && savedLayouts().length + chosenLayouts.length > MAX_SAVED) { setProblem(t('pack-error-layouts-full', { max: MAX_SAVED })); return; }
    const m = withKeys ? merge(pack) : undefined;
    if (m && m.issues.length > 0) { setProblem(t('pack-error-keys-merge', { text: m.issues.map(keyIssueText).join(' ') })); return; }
    const lines: string[] = [];
    if (chosenThemes.length > 0) {
      const r = store.add(chosenThemes);
      if (!r.ok) { setProblem('full' in r ? fullText() : r.issues.map(issueText).join(' ')); return; }
      lines.push(t('pack-done-themes', { count: r.themes.length }));
    }
    if (chosenLayouts.length > 0) {
      const r = addNamed(chosenLayouts, (name, n) => t('pack-layout-copy', { name, n }));
      if (!r.ok) { setProblem(t('pack-error-layouts-full', { max: MAX_SAVED })); return; }
      lines.push(t('pack-done-layouts', { count: r.names.length, names: r.names.join(', ') }));
    }
    if (m) { keys.set(m.file, m.unknown); lines.push(t('pack-done-keys', { count: pack.keys?.file.entries.length ?? 0 })); }
    // Kept as the file it was (read again with the pack rules), once.
    try { const file: unknown = JSON.parse(text); if (file && typeof file === 'object' && !Array.isArray(file)) packs.keep(file as Record<string, unknown>); } catch { /* the text was read above */ }
    setStep({ at: 'done', lines });
  };

  if (step.at === 'done') {
    return (
      <Shell title={t('pack-import-title')} icon="upload" onClose={onClose} label="done">
        <ul role="status" className="m-0 grid list-none gap-1 p-0">{step.lines.map((l) => <li key={l} className="flex items-start gap-2"><Icon name="check" className="mt-0.5 size-3.5 text-finished" />{l}</li>)}</ul>
        <div className="kit-dialog__actions"><Button variant="primary" onClick={onClose}>{t('pack-close')}</Button></div>
      </Shell>
    );
  }

  if (step.at === 'keys' && step.pack.keys) {
    const m = merge(step.pack);
    const label = (id: string) => { const d = commandById(id); return d ? commandLabel(d) : id; };
    const keysText = (ks: readonly string[]) => (ks.length === 0 ? t('keys-no-keys') : ks.map((k) => bindingLabel(parseBinding(k) ?? [], mac)).join(', '));
    return (
      <Shell title={t('pack-keys-confirm-title')} icon="upload" onClose={onClose} label="keys">
        <p className="muted m-0 text-xs">{t('pack-keys-confirm-hint', { count: step.pack.keys.file.entries.length })}</p>
        {m && m.replaced.length === 0 && m.clashes.length === 0 && <p className="m-0 text-xs" data-pack-no-conflicts>{t('pack-keys-no-conflicts')}</p>}
        {m && m.replaced.length > 0 && <Section title={t('pack-keys-replaced')}>
          <ul className="m-0 grid list-none gap-1 p-0 text-xs" data-pack-replaced>
            {m.replaced.map((r) => <li key={r.command}>{t('pack-keys-replaced-row', { name: label(r.command), mine: keysText(r.mine), theirs: keysText(r.theirs) })}</li>)}
          </ul>
        </Section>}
        {m && m.clashes.length > 0 && <Section title={t('pack-keys-clashes')}>
          <ul className="m-0 grid list-none gap-1 p-0 text-xs" data-pack-clashes>
            {m.clashes.map((c) => <li key={`${c.a} ${c.b} ${c.keys}`} className="flex items-start gap-1.5 text-danger"><Icon name="warn" className="mt-0.5 size-3.5 flex-none" />
              {t(c.kind === 'same' ? 'pack-keys-clash-same' : 'pack-keys-clash-prefix', { key: bindingLabel(parseBinding(c.keys) ?? [], mac), a: label(c.a), b: label(c.b) })}</li>)}
          </ul>
        </Section>}
        {problem && <p role="alert" className="m-0 text-xs text-danger">{problem}</p>}
        <div className="kit-dialog__actions">
          <Button onClick={() => { setProblem(undefined); setStep({ at: 'preview', pack: step.pack }); }}>{t('pack-back')}</Button>
          <Button variant="primary" onClick={() => apply(step.pack)}>{t('pack-keys-merge-do')}</Button>
        </div>
      </Shell>
    );
  }

  if (step.at === 'preview' || step.at === 'keys') {
    const { pack } = step;
    const room = store.room();
    const nothing = themes.size === 0 && layouts.size === 0 && !withKeys;
    return (
      <Shell title={t('pack-preview-title', { name: pack.name })} icon="upload" onClose={onClose} label="preview">
        <p className="muted m-0 text-xs">{t('pack-preview-hint')}</p>
        {pack.themes.length > 0 && <Section title={t('pack-themes')}>
          <div className="pack-list">
            {pack.themes.map((x, i) => <Check key={i} checked={themes.has(i)} onChange={(on) => { setProblem(undefined); setThemes((s) => toggled(s, i, on)); }}
              title={x.theme.name} meta={`${schemesText(schemesOf(x.theme))}${x.warnings.length > 0 ? ` · ${t('pack-theme-warnings', { count: x.warnings.length })}` : ''}`} />)}
          </div>
          {themes.size > room && <p className="m-0 text-xs text-danger">{t('pack-themes-room', { count: room })}</p>}
        </Section>}
        {pack.layouts.length > 0 && <Section title={t('pack-layouts')}>
          <div className="pack-list">
            {pack.layouts.map((l, i) => {
              const miss = missingPanels(l.layout, knownPanel);
              return (
                <Check key={l.name} checked={layouts.has(i)} onChange={(on) => { setProblem(undefined); setLayouts((s) => toggled(s, i, on)); }}
                  title={l.name} meta={t('pack-panes', { count: panesOf(l.layout.root).length })}>
                  {miss.plugins.length > 0 && <span className="text-xs text-muted" data-pack-plugin-panels>{t('pack-layout-plugin', { panels: miss.plugins.join(', ') })}</span>}
                  {miss.unknown.length > 0 && <span className="text-xs text-muted">{t('pack-layout-unknown', { panels: miss.unknown.join(', ') })}</span>}
                </Check>
              );
            })}
          </div>
        </Section>}
        {pack.keys && <Section title={t('pack-keys')}>
          <Check checked={withKeys} onChange={(on) => { setProblem(undefined); setWithKeys(on); }} title={t('pack-keys-theirs')}
            meta={t('pack-keys-count', { count: pack.keys.file.entries.length })}>
            {pack.keys.unknown.length > 0 && <span className="text-xs text-muted">{t('pack-keys-unknown', { count: pack.keys.unknown.length })}</span>}
          </Check>
        </Section>}
        {problem && <p role="alert" className="m-0 text-xs text-danger">{problem}</p>}
        <div className="kit-dialog__actions">
          <Button onClick={() => setStep({ at: 'read' })}>{t('pack-back')}</Button>
          <Button variant="primary" disabled={nothing} onClick={() => (withKeys ? setStep({ at: 'keys', pack }) : apply(pack))}>
            {withKeys ? t('pack-next') : t('pack-apply')}
          </Button>
        </div>
      </Shell>
    );
  }

  return (
    <Shell title={t('pack-import-title')} icon="upload" onClose={onClose} label="read">
      <p className="muted m-0 text-xs">{t('pack-import-hint')}</p>
      <label className="grid gap-1">{t('pack-import-paste')}
        <textarea className="min-h-32 rounded-s border border-line bg-surface-0 p-2 font-mono text-xs text-ink" value={text} spellCheck={false}
          aria-invalid={issues.length > 0} aria-describedby={issues.length ? 'pack-import-problems' : undefined} onChange={(e) => { setText(e.target.value); setIssues([]); }} />
      </label>
      <label className="grid gap-1">{t('pack-import-file')}
        <input type="file" accept=".json,application/json" className="text-xs" onChange={(e) => {
          const f = e.target.files?.[0];
          e.target.value = '';
          if (!f) return;
          // Refused before it is read: a file over the cap is never loaded into the page.
          if (f.size > MAX_PACK_BYTES) { setIssues([{ code: 'too-large', max: MAX_PACK_BYTES / 1024 }]); return; }
          void f.text().then((s) => { setText(s); setIssues([]); }).catch(() => setIssues([{ code: 'json' }]));
        }} />
      </label>
      {issues.length > 0 && (
        <div id="pack-import-problems" role="alert" className="grid gap-1 text-xs text-danger">
          <p className="m-0 font-medium">{t('pack-import-refused')}</p>
          <ul className="m-0 grid gap-0.5 ps-5">{issues.map((i, n) => <li key={n}>{packIssueText(i)}</li>)}</ul>
        </div>
      )}
      <div className="kit-dialog__actions">
        <Button onClick={onClose}>{t('layout-cancel')}</Button>
        <Button variant="primary" disabled={!text.trim()} onClick={preview}>{t('pack-preview-do')}</Button>
      </div>
    </Shell>
  );
}
