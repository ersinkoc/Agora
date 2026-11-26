// Plugins loaded for development, and the install screen (ADR-0114 decisions 3 and 6; docs/ux/plugins.md). Opened from the command
// palette ("Plugins"). A package is loaded from a local file into this browser only; the install screen lists what it asks to see in
// plain words and the person allows each one. Everything a plugin's name, author or licence says is shown as text, never as the window's
// own words.
import * as Dialog from '@radix-ui/react-dialog';
import { useId, useRef, useState } from 'react';
import { t } from '../i18n';
import { Button } from '../components/kit';
import { PluginMark } from '../layout/Workspace';
import { type Capability, capabilityKey } from './capabilities';
import type { CheckedPlugin, PackageError } from './package';
import { unsafeAddress } from './safe';
import { pluginStore } from './store';
import { usePlugins } from './usePlugins';

// eslint-disable-next-line no-control-regex
const plain = (s: string) => s.replace(/[\u0000-\u001f\u007f-\u009f​-‏‪-‮⁠-⁯﻿]/g, '').slice(0, 100);
export const packageErrorText = (code: PackageError | 'full', detail?: string) => t(`plugin-error-${code}`, { detail: plain(detail ?? '') });

type Screen =
  | { at: 'list'; error?: string; note?: string }
  | { at: 'grant'; plugin: CheckedPlugin; text?: string; allow: Capability[] };

export function PluginsDialog({ onClose }: { onClose: () => void }) {
  const { plugins, safe } = usePlugins();
  const [screen, setScreen] = useState<Screen>({ at: 'list' });
  const file = useRef<HTMLInputElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const titleId = useId();

  const choose = async (f: File | undefined) => {
    if (!f) return;
    const r = await pluginStore.check(f);
    if (file.current) file.current.value = '';
    if (!r.ok) { setScreen({ at: 'list', error: packageErrorText(r.code, r.detail) }); return; }
    // A plugin loaded again keeps what the person allowed it, for the capabilities it still asks for.
    const before = plugins.find((p) => p.plugin.manifest.id === r.plugin.manifest.id)?.grant?.granted ?? [];
    setScreen({ at: 'grant', plugin: r.plugin, text: r.text, allow: r.plugin.manifest.capabilities.filter((c) => before.includes(c)) });
  };
  const save = (s: Extract<Screen, { at: 'grant' }>) => {
    if (s.text === undefined) { pluginStore.regrant(s.plugin.manifest.id, s.allow); setScreen({ at: 'list' }); return; }
    const kept = pluginStore.install(s.plugin, s.text, s.allow);
    if (kept === 'full') { setScreen({ at: 'list', error: packageErrorText('full') }); return; }
    setScreen({ at: 'list', note: t(kept === 'kept' ? 'plugin-added' : 'plugin-added-visit', { name: s.plugin.manifest.name }) });
  };

  return (
    <Dialog.Root open onOpenChange={(open) => { if (!open) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-40 bg-surface-0/70 backdrop-blur-sm" />
        {/* The dialog itself takes the focus when it opens, never one of its buttons: the Enter that chose "Plugins" in the palette
            must not press Remove or Done. */}
        <Dialog.Content aria-labelledby={titleId} aria-describedby={undefined} data-plugins-dialog="" tabIndex={-1}
          ref={content} onOpenAutoFocus={(e) => { e.preventDefault(); content.current?.focus(); }}
          className="fixed left-1/2 top-1/2 z-50 grid outline-none max-h-[calc(100dvh-2rem)] w-[min(32rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 gap-3 overflow-y-auto rounded-l border border-line bg-surface-1 p-5 shadow-card">
          {screen.at === 'list' ? <>
            <Dialog.Title id={titleId} className="m-0 text-base font-semibold normal-case tracking-normal text-ink">{t('plugins-title')}</Dialog.Title>
            <p className="m-0 text-muted">{t(safe ? 'plugins-safe' : 'plugins-intro')}</p>
            {/* In safe mode no plugin runs; what this browser keeps is listed by name only, to be removed. */}
            {safe && pluginStore.kept().length > 0 && <ul className="m-0 grid list-none gap-2 p-0">
              {pluginStore.kept().map((k) => (
                <li key={k.id} className="flex min-w-0 flex-wrap items-center gap-2 rounded-m border border-line p-3" data-plugin-row={k.id}>
                  <span className="min-w-0 flex-1 truncate font-medium text-ink">{k.name}</span>
                  <span className="text-xs text-muted">{k.version}</span>
                  <Button size="sm" variant="danger-ghost" onClick={() => pluginStore.remove(k.id)}>{t('plugin-remove')}</Button>
                </li>
              ))}
            </ul>}
            {plugins.length > 0 && <ul className="m-0 grid list-none gap-2 p-0">
              {plugins.map((p) => {
                const m = p.plugin.manifest;
                return (
                  <li key={m.id} className="grid gap-1.5 rounded-m border border-line p-3" data-plugin-row={m.id}>
                    <div className="flex min-w-0 flex-wrap items-center gap-2">
                      <span className="min-w-0 truncate font-medium text-ink">{m.name}</span>
                      <PluginMark dev={p.dev} />
                      <span className="text-xs text-muted">{t('plugin-version-by', { version: m.version, author: m.author })}</span>
                    </div>
                    <p className="m-0 text-xs text-muted">{p.state === 'held' ? t('plugin-held') : t('plugin-sees', { count: p.granted.length })}</p>
                    <div className="flex flex-wrap gap-2">
                      <Button size="sm" onClick={() => setScreen({ at: 'grant', plugin: p.plugin, allow: [...p.granted] })}>{t(p.state === 'held' ? 'plugin-review' : 'plugin-change')}</Button>
                      <Button size="sm" variant="danger-ghost" onClick={() => pluginStore.remove(m.id)}>{t('plugin-remove')}</Button>
                    </div>
                  </li>
                );
              })}
            </ul>}
            {screen.error && <p role="alert" className="m-0 text-danger">{screen.error}</p>}
            {screen.note && <p role="status" className="m-0">{screen.note}</p>}
            <div className="mt-1 flex flex-wrap justify-end gap-2">
              {safe ? <Button onClick={() => location.assign(unsafeAddress(location.href))}>{t('safe-mode-leave')}</Button>
                : <><input ref={file} type="file" accept=".json,application/json" className="sr-only" tabIndex={-1} aria-hidden data-plugin-file="" onChange={(e) => void choose(e.target.files?.[0])} />
                  <Button onClick={() => file.current?.click()}>{t('plugin-load')}</Button></>}
              <Button variant="primary" onClick={onClose}>{t('plugins-done')}</Button>
            </div>
          </> : <Grant titleId={titleId} screen={screen} onChange={(allow) => setScreen({ ...screen, allow })} onCancel={() => setScreen({ at: 'list' })} onSave={() => save(screen)} />}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

// The install screen: who made it, that it was loaded for development, what it can never do, and each thing it asks to see.
function Grant({ titleId, screen, onChange, onCancel, onSave }: { titleId: string; screen: Extract<Screen, { at: 'grant' }>; onChange: (allow: Capability[]) => void; onCancel: () => void; onSave: () => void }) {
  const m = screen.plugin.manifest;
  const toggle = (c: Capability, on: boolean) => onChange(m.capabilities.filter((x) => (x === c ? on : screen.allow.includes(x))));
  return <>
    <Dialog.Title id={titleId} className="m-0 text-base font-semibold normal-case tracking-normal text-ink">{t('plugin-grant-title', { name: m.name })}</Dialog.Title>
    <div className="flex flex-wrap items-center gap-2 text-xs text-muted">
      <PluginMark dev />
      <span>{t('plugin-version-by', { version: m.version, author: m.author })}</span>
      <span>{t('plugin-licence', { licence: m.licence })}</span>
    </div>
    <p className="m-0 text-muted">{t('plugin-grant-never')}</p>
    {m.capabilities.length === 0 ? <p className="m-0">{t('plugin-grant-nothing')}</p> : <fieldset className="m-0 grid gap-2 border-0 p-0" data-plugin-grants="">
      <legend className="mb-1 p-0 font-medium text-ink">{t('plugin-grant-asks')}</legend>
      {m.capabilities.map((c) => (
        <label key={c} className="flex items-start gap-2">
          <input type="checkbox" className="mt-0.5 size-4 flex-none" checked={screen.allow.includes(c)} onChange={(e) => toggle(c, e.target.checked)} data-cap={c} />
          <span>{t(capabilityKey(c))}</span>
        </label>
      ))}
    </fieldset>}
    <div className="mt-1 flex flex-wrap justify-end gap-2">
      <Button onClick={onCancel}>{t('plugin-cancel')}</Button>
      <Button variant="primary" onClick={onSave} data-plugin-save="">{t(screen.text === undefined ? 'plugin-save' : 'plugin-add')}</Button>
    </div>
  </>;
}

/** In safe mode, a note at the bottom of the window that says so, with the way out. */
export function SafeModeNote() {
  return (
    <div role="status" data-safe-mode="" className="fixed bottom-8 start-1/2 z-30 flex -translate-x-1/2 items-center gap-3 rounded-full border border-line bg-surface-1 py-1.5 pe-1.5 ps-4 text-xs text-ink shadow-card rtl:translate-x-1/2">
      <span>{t('safe-mode-on')}</span>
      <Button size="sm" onClick={() => location.assign(unsafeAddress(location.href))}>{t('safe-mode-leave')}</Button>
    </div>
  );
}
