// Settings > Keyboard (ADR-0068 rung 4, docs/ux/keybindings.md): every command with its keys, a change by pressing the new keys (a chord
// is two presses in a row), the keys a command shares with another, and the keys that stay as they are. The person's changes are one
// keybindings file in this browser, which can be exported and imported; a file from elsewhere goes through the same checks as a stored one.
import * as Dialog from '@radix-ui/react-dialog';
import { useEffect, useMemo, useRef, useState, useSyncExternalStore, type KeyboardEvent as ReactKeyboardEvent } from 'react';
import { t } from '../i18n';
import { Icon } from '../components/Icon';
import { SettingsBlock, SettingsCard } from '../components/SettingsCard';
import { Button, ConfirmDialog, Pill } from '../components/kit';
import { COMMANDS, FIXED_KEYS, type CommandDef, type FixedReason, commandById, fixedReason } from './commands';
import { type Conflict, type KeyIssue, MAX_FILE_BYTES, conflicts, keysOf, readKeys, withKeys, writeKeys } from './file';
import { CHORD_MS } from './registry';
import { type KeyStore, keyStore } from './store';
import { IS_MAC, type Stroke, bindingLabel, bindingText, parseBinding, strokeLabel, strokeOf } from './strokes';
import { commandLabel } from './view';

const FIXED_REASONS: readonly FixedReason[] = ['safe-mode', 'navigation', 'terminal', 'editing'];

/** A problem of a keybindings file in words. */
export function issueText(i: KeyIssue): string {
  switch (i.code) {
    case 'field': return t('keys-error-field', { name: i.at });
    case 'command': case 'request': case 'fixed-command': case 'twice': case 'too-many-keys': return t(`keys-error-${i.code}`, { command: i.command });
    case 'key': return t('keys-error-key', { command: i.command, key: i.key });
    case 'fixed-key': return t('keys-error-fixed-key', { command: i.command, key: i.key, reason: t(`keys-fixed-${i.reason}`) });
    case 'more': return t('keys-error-more', { count: i.count });
    default: return t(`keys-error-${i.code}`);
  }
}

function download(text: string) {
  try {
    const url = URL.createObjectURL(new Blob([text], { type: 'application/json' }));
    const a = document.createElement('a');
    a.href = url;
    a.download = 'keybindings.json';
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  } catch { /* nothing to save to here */ }
}

const Keys = ({ labels }: { labels: readonly string[] }) => <span className="keys-caps">{labels.map((l) => <kbd key={l} className="ide-kbd">{l}</kbd>)}</span>;

export function KeyboardSettings({ store = keyStore, mac = IS_MAC }: { store?: KeyStore; mac?: boolean }) {
  const state = useSyncExternalStore(store.subscribe, store.state, store.state);
  const [query, setQuery] = useState('');
  const [recording, setRecording] = useState<{ id: string; first?: Stroke } | undefined>();
  const [refused, setRefused] = useState<{ id: string; text: string } | undefined>();
  const [importing, setImporting] = useState(false);
  const [resetting, setResetting] = useState(false);
  const timer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(timer.current), []);
  const clash = useMemo(() => conflicts(state.map), [state.map]);
  const own = useMemo(() => new Set(state.file.entries.map((e) => e.command)), [state.file]);
  const label = (id: string) => { const d = commandById(id); return d ? commandLabel(d) : id; };
  const rows = useMemo(() => {
    const q = query.trim().toLowerCase();
    return COMMANDS.filter((d) => {
      if (!q) return true;
      const keys = keysOf(state.map, d.id).map((b) => bindingLabel(b, mac)).join(' ');
      return `${commandLabel(d)} ${d.id} ${keys}`.toLowerCase().includes(q);
    });
  }, [query, state.map, mac]); // eslint-disable-line react-hooks/exhaustive-deps

  const save = (id: string, keys: readonly string[] | undefined) => { setRefused(undefined); store.set(withKeys(state.file, id, keys)); };
  const finish = (id: string, strokes: readonly Stroke[]) => {
    window.clearTimeout(timer.current);
    setRecording(undefined);
    const fixed = strokes.map(fixedReason).find((r) => r !== undefined);
    const text = bindingText(strokes);
    if (fixed) { setRefused({ id, text: t('keys-refused-fixed', { key: bindingLabel(strokes, mac), reason: t(`keys-fixed-${fixed}`) }) }); return; }
    save(id, [text]);
  };
  // While recording, the keys go to the recorder only: a command's key, a terminal's or the window's own does not act (the safe-mode key,
  // heard before anything else, still does). Tab leaves, Esc stops.
  const record = (id: string) => (e: ReactKeyboardEvent<HTMLButtonElement>) => {
    if (e.key === 'Tab' && !e.ctrlKey && !e.altKey && !e.metaKey) { setRecording(undefined); return; }
    e.preventDefault();
    e.stopPropagation();
    if (e.key === 'Escape') { window.clearTimeout(timer.current); setRecording(undefined); return; }
    const s = strokeOf(e.nativeEvent);
    if (!s) return;
    const first = recording?.id === id ? recording.first : undefined;
    if (first) { finish(id, [first, s]); return; }
    setRecording({ id, first: s });
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => finish(id, [s]), CHORD_MS);
  };

  const clashesOf = (id: string): Conflict[] => clash.filter((c) => c.a === id || c.b === id);
  return (
    <SettingsCard id="keyboard" icon="keyboard" title={t('keys-title')} description={t('keys-desc')}
      end={own.size > 0 ? <Pill tone="plain">{t('keys-changed-count', { count: own.size })}</Pill> : undefined}>
      <div className="keys-bar">
        <label className="keys-search">
          <Icon name="search" className="size-3.5 flex-none text-muted" />
          <input type="search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder={t('keys-search')} aria-label={t('keys-search')} />
        </label>
        <Button size="sm" variant="ghost" onClick={() => setImporting(true)}><Icon name="upload" className="size-3.5" />{t('keys-import')}</Button>
        <Button size="sm" variant="ghost" onClick={() => download(writeKeys(state.file))}><Icon name="download" className="size-3.5" />{t('keys-export')}</Button>
        <Button size="sm" variant="ghost" disabled={own.size === 0} onClick={() => setResetting(true)}>{t('keys-reset-all')}</Button>
      </div>
      {state.unreadable && <SettingsBlock><p role="status" className="m-0">{t('keys-unreadable')}</p></SettingsBlock>}
      {state.unknown.length > 0 && (
        <SettingsBlock>
          <p className="m-0 text-muted">{t('keys-unknown', { count: state.unknown.length })}</p>
          <p className="m-0 keys-unknown">{state.unknown.map((u) => <code key={u}>{u}</code>)}</p>
        </SettingsBlock>
      )}
      <ul className="set-list keys-list" aria-label={t('keys-list')}>
        {rows.map(row)}
        {rows.length === 0 && <li className="set-item text-muted">{t('keys-none-match')}</li>}
      </ul>
      <SettingsBlock>
        <p className="m-0 font-medium">{t('keys-fixed-title')}</p>
        <dl className="keys-fixed">
          {FIXED_REASONS.map((r) => (
            <div key={r} className="contents">
              <dt>{t(`keys-fixed-${r}`)}</dt>
              <dd><Keys labels={FIXED_KEYS.filter((f) => f.reason === r && (r !== 'safe-mode' || (f.keys.includes('meta') === mac))).map((f) => bindingLabel(parseBinding(f.keys) ?? [], mac))} /></dd>
            </div>
          ))}
        </dl>
        <p className="m-0 text-muted">{t('keys-terminal-note')}</p>
      </SettingsBlock>
      {importing && <ImportKeys onImport={(file, unknown) => { store.set(file, unknown); setImporting(false); }} onClose={() => setImporting(false)} />}
      <ConfirmDialog open={resetting} icon="history" title={t('keys-reset-title')} body={t('keys-reset-body')} cancel={t('keys-reset-keep')} confirm={t('keys-reset-confirm')}
        onCancel={() => setResetting(false)} onConfirm={() => { setResetting(false); setRefused(undefined); store.reset(); }} />
    </SettingsCard>
  );

  // A function, not a component: a component defined in here would be a new type on every render, and the row being recorded would lose
  // the keyboard.
  function row(def: CommandDef) {
    const keys = keysOf(state.map, def.id);
    const mine = own.has(def.id);
    const others = clashesOf(def.id);
    const live = recording?.id === def.id;
    return (
      <li key={def.id} className="set-item keys-row" data-command={def.id} data-conflict={others.length > 0 || undefined}>
        <span className="set-item__main">
          <span className="set-item__name">{commandLabel(def)}</span>
          <span className="set-item__meta"><code>{def.id}</code><span>{t(`keys-context-${def.context}`)}</span>{mine && <span className="set-item__flag">{t('keys-changed')}</span>}</span>
        </span>
        <span className="set-item__end">
          {live ? <span className="keys-live" aria-live="polite">{recording?.first ? t('keys-press-second', { key: strokeLabel(recording.first, mac) }) : t('keys-press')}</span>
            : keys.length ? <Keys labels={keys.map((b) => bindingLabel(b, mac))} /> : <span className="text-muted">{t('keys-no-keys')}</span>}
          {def.fixed ? <Pill tone="plain" icon={<Icon name="lock" className="size-3" />}>{t('keys-fixed')}</Pill> : <>
            <Button size="sm" variant={live ? 'primary' : 'ghost'} aria-label={t('keys-change-named', { name: commandLabel(def) })} autoFocus={live}
              onClick={() => { setRefused(undefined); setRecording(live ? undefined : { id: def.id }); }} onKeyDown={live ? record(def.id) : undefined} onBlur={() => { if (live) { window.clearTimeout(timer.current); setRecording(undefined); } }}>
              {live ? t('keys-recording') : t('keys-change')}
            </Button>
            {keys.length > 0 && <Button size="sm" variant="ghost" aria-label={t('keys-clear-named', { name: commandLabel(def) })} onClick={() => save(def.id, [])}>{t('keys-clear')}</Button>}
            {mine && <Button size="sm" variant="ghost" aria-label={t('keys-reset-named', { name: commandLabel(def) })} onClick={() => save(def.id, undefined)}>{t('keys-reset')}</Button>}
          </>}
        </span>
        {others.length > 0 && (
          <p className="keys-clash" role="note">
            <Icon name="warn" className="size-3.5 flex-none" />
            {others.map((c) => t(c.kind === 'same' ? 'keys-conflict-same' : 'keys-conflict-prefix', { key: bindingLabel(parseBinding(c.keys) ?? [], mac), name: label(c.a === def.id ? c.b : c.a) })).join(' ')}
          </p>
        )}
        {refused?.id === def.id && <p role="alert" className="set-item__alert">{refused.text}</p>}
      </li>
    );
  }
}

function ImportKeys({ onImport, onClose }: { onImport: (file: import('./file').KeyFile, unknown: string[]) => void; onClose: () => void }) {
  const [text, setText] = useState('');
  const [issues, setIssues] = useState<KeyIssue[]>([]);
  const submit = () => { const r = readKeys(text); if (r.ok) onImport(r.file, r.unknown); else setIssues(r.issues); };
  return (
    <Dialog.Root open onOpenChange={(open) => { if (!open) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="kit-overlay" />
        <Dialog.Content aria-describedby={undefined} className="kit-dialog keys-import">
          <Dialog.Title className="kit-dialog__title">{t('keys-import-title')}</Dialog.Title>
          <label className="grid gap-1">{t('keys-import-paste')}
            <textarea className="min-h-40 rounded-s border border-line bg-surface-0 p-2 font-mono text-xs text-ink" value={text} spellCheck={false}
              aria-invalid={issues.length > 0} aria-describedby={issues.length ? 'keys-import-problems' : undefined} onChange={(e) => { setText(e.target.value); setIssues([]); }} />
          </label>
          <label className="grid gap-1">{t('keys-import-file')}
            <input type="file" accept=".json,application/json" className="text-xs" onChange={(e) => {
              const f = e.target.files?.[0];
              if (!f) return;
              // Refused before it is read: a file over the cap is never loaded into the page.
              if (f.size > MAX_FILE_BYTES) { setIssues([{ code: 'too-big' }]); return; }
              void f.text().then((s) => { setText(s); setIssues([]); }).catch(() => setIssues([{ code: 'json' }]));
            }} />
          </label>
          {issues.length > 0 && (
            <div id="keys-import-problems" role="alert" className="grid gap-1 text-danger">
              <p className="m-0 font-medium">{t('keys-import-refused')}</p>
              <ul className="m-0 grid gap-0.5 ps-5">{issues.map((i, n) => <li key={n}>{issueText(i)}</li>)}</ul>
            </div>
          )}
          <div className="kit-dialog__actions">
            <Button onClick={onClose}>{t('layout-cancel')}</Button>
            <Button variant="primary" disabled={!text.trim()} onClick={submit}>{t('layout-import-do')}</Button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
