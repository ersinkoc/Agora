// The plugins of this window, and the install screen (ADR-0114 decisions 3 and 6; docs/ux/plugins.md). Opened from the command palette
// ("Plugins"). It lists the plugins installed on the hub (each person chooses whether to show one, and what it may see; only the hub's
// owner adds or removes them, on the hub's machine) and those loaded from a local file into this browser only, for development. The
// install screen lists what a plugin asks to see in plain words and the person allows each one. Everything a plugin's name, author or
// licence says is shown as text, never as the window's own words. When the hub's owner has the community catalog on, its plugins not
// installed yet (or newer than the installed version) are listed read only, with the command the owner runs on the hub's machine:
// nothing is installed from a window (ADR-0114 decisions 6 and 7). A development plugin's file chosen with the picker that lets this
// page read it again is watched for this visit (reload on save, devReload.ts).
import * as Dialog from '@radix-ui/react-dialog';
import { useId, useRef, useState } from 'react';
import { t } from '../i18n';
import { Button } from '../components/kit';
import { PluginMark } from '../layout/Workspace';
import { type Capability, capabilityKey } from './capabilities';
import { type ReloadProblem, type WatchedFile, canWatchFiles, devReload, pickPackage, useDevReload } from './devReload';
import { type CatalogPlugin, compareVersions } from './hub';
import type { CheckedPlugin, PackageError } from './package';
import { unsafeAddress } from './safe';
import { pluginStore } from './store';
import { usePlugins } from './usePlugins';

// eslint-disable-next-line no-control-regex
const plain = (s: string) => s.replace(/[\u0000-\u001f\u007f-\u009f​-‏‪-‮⁠-⁯﻿]/g, '').slice(0, 100);
export const packageErrorText = (code: PackageError | 'full', detail?: string) => t(`plugin-error-${code}`, { detail: plain(detail ?? '') });
/** Why a development plugin's changed file was not loaded (reload on save). */
export const reloadProblemText = (name: string, p: ReloadProblem): string => t('plugin-reload-refused', {
  name,
  reason: p.code === 'other-id' ? t('plugin-reload-other-id', { id: plain(p.detail ?? '') })
    : p.code === 'unreadable' ? t('plugin-reload-unreadable')
      : packageErrorText(p.code, p.detail),
});

type Screen =
  | { at: 'list'; error?: string; note?: string }
  | { at: 'grant'; plugin: CheckedPlugin; text?: string; allow: Capability[]; dev: boolean; fresh?: boolean; watch?: { handle: WatchedFile; file: Pick<File, 'size' | 'lastModified'> } };

export function PluginsDialog({ onClose }: { onClose: () => void }) {
  const { plugins, safe, replaced, catalog } = usePlugins();
  const reload = useDevReload();
  // The hub plugin a development plugin of this id stands in for (or would), by its name.
  const hubTwin = (id: string) => [...replaced, ...plugins].find((p) => !p.dev && p.plugin.manifest.id === id)?.plugin.manifest.name;
  const [screen, setScreen] = useState<Screen>({ at: 'list' });
  const file = useRef<HTMLInputElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const titleId = useId();

  const choose = async (f: File | undefined, handle?: WatchedFile) => {
    if (!f) return;
    const r = await pluginStore.check(f);
    if (file.current) file.current.value = '';
    if (!r.ok) { setScreen({ at: 'list', error: packageErrorText(r.code, r.detail) }); return; }
    // A plugin loaded again keeps what the person allowed it, for the capabilities it still asks for.
    // A development plugin never takes a hub plugin's grant, even with the same id.
    const before = plugins.find((p) => p.dev && p.plugin.manifest.id === r.plugin.manifest.id)?.grant?.granted ?? [];
    setScreen({ at: 'grant', plugin: r.plugin, text: r.text, allow: r.plugin.manifest.capabilities.filter((c) => before.includes(c)), dev: true, ...(handle ? { watch: { handle, file: f } } : {}) });
  };
  // The picker that lets this page read the file again where there is one (reload on save), else the file input.
  const load = async () => {
    if (!canWatchFiles()) { file.current?.click(); return; }
    const picked = await pickPackage(t('plugin-file-kind'));
    if (picked === 'failed') file.current?.click();
    else if (picked) await choose(picked.file, picked.handle);
  };
  const save = async (s: Extract<Screen, { at: 'grant' }>) => {
    if (s.text === undefined) {
      const kept = await pluginStore.regrant(s.plugin.manifest.id, s.allow);
      setScreen(kept ? { at: 'list' } : { at: 'list', error: t('plugin-grant-failed') });
      return;
    }
    const kept = pluginStore.install(s.plugin, s.text, s.allow);
    if (kept === 'full') { setScreen({ at: 'list', error: packageErrorText('full') }); return; }
    if (s.watch) devReload.watch(s.plugin.manifest.id, s.watch.handle, s.watch.file); else devReload.unwatch(s.plugin.manifest.id);
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
                      {!p.dev && <span className="text-xs text-muted" data-plugin-hub="">{t('plugin-from-hub')}</span>}
                    </div>
                    <p className="m-0 text-xs text-muted">{p.state === 'new' ? t('plugin-not-shown') : p.state === 'held' ? t('plugin-held') : t('plugin-sees', { count: p.granted.length })}</p>
                    {p.dev && reload.watching.includes(m.id) && <p className="m-0 text-xs text-muted" data-plugin-reload={m.id}>{t('plugin-reload-on')}</p>}
                    {p.dev && reload.problems[m.id] && <p role="alert" className="m-0 text-xs text-danger">{reloadProblemText(m.name, reload.problems[m.id]!)}</p>}
                    <div className="flex flex-wrap gap-2">
                      <Button size="sm" onClick={() => setScreen({ at: 'grant', plugin: p.plugin, allow: [...p.granted], dev: p.dev, fresh: p.state === 'new' })}>{t(p.state === 'new' ? 'plugin-look' : p.state === 'held' ? 'plugin-review' : 'plugin-change')}</Button>
                      {/* A hub plugin is the owner's to remove, on the hub's machine; a person can stop showing it. */}
                      {p.dev ? <Button size="sm" variant="danger-ghost" onClick={() => pluginStore.remove(m.id)}>{t('plugin-remove')}</Button>
                        : p.state !== 'new' && <Button size="sm" variant="danger-ghost" onClick={() => void pluginStore.hide(m.id).then((ok) => { if (!ok) setScreen({ at: 'list', error: t('plugin-grant-failed') }); })}>{t('plugin-hide')}</Button>}
                    </div>
                  </li>
                );
              })}
            </ul>}
            {/* A hub plugin a development plugin stands in for here: still listed, marked as replaced in this browser, never run. */}
            {replaced.length > 0 && <ul className="m-0 grid list-none gap-2 p-0">
              {replaced.map((p) => (
                <li key={`hub:${p.plugin.manifest.id}`} className="grid gap-1.5 rounded-m border border-line p-3" data-plugin-row={p.plugin.manifest.id} data-plugin-replaced="">
                  <div className="flex min-w-0 flex-wrap items-center gap-2">
                    <span className="min-w-0 truncate font-medium text-ink">{p.plugin.manifest.name}</span>
                    <PluginMark dev={false} />
                    <span className="text-xs text-muted">{t('plugin-version-by', { version: p.plugin.manifest.version, author: p.plugin.manifest.author })}</span>
                    <span className="text-xs text-muted">{t('plugin-from-hub')}</span>
                  </div>
                  <p className="m-0 text-xs text-muted">{t('plugin-replaced')}</p>
                </li>
              ))}
            </ul>}
            {!safe && catalog?.on && <CatalogList entries={catalog.entries} installed={(id) => [...plugins, ...replaced].find((p) => !p.dev && p.plugin.manifest.id === id)?.plugin.manifest.version} />}
            {!safe && !canWatchFiles() && <p className="m-0 text-xs text-muted" data-plugin-reload-unsupported="">{t('plugin-reload-unsupported')}</p>}
            {screen.error && <p role="alert" className="m-0 text-danger">{screen.error}</p>}
            {screen.note && <p role="status" className="m-0">{screen.note}</p>}
            <div className="mt-1 flex flex-wrap justify-end gap-2">
              {safe ? <Button onClick={() => location.assign(unsafeAddress(location.href))}>{t('safe-mode-leave')}</Button>
                : <><input ref={file} type="file" accept=".json,application/json" className="sr-only" tabIndex={-1} aria-hidden data-plugin-file="" onChange={(e) => void choose(e.target.files?.[0])} />
                  <Button onClick={() => void load()}>{t('plugin-load')}</Button></>}
              <Button variant="primary" onClick={onClose}>{t('plugins-done')}</Button>
            </div>
          </> : <Grant titleId={titleId} screen={screen} onChange={(allow) => setScreen({ ...screen, allow })} onCancel={() => setScreen({ at: 'list' })} onSave={() => void save(screen)} twin={screen.dev ? hubTwin(screen.plugin.manifest.id) : undefined} />}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

// The install screen: who made it, that it was loaded for development, what it can never do, and each thing it asks to see.
function Grant({ titleId, screen, onChange, onCancel, onSave, twin }: { titleId: string; screen: Extract<Screen, { at: 'grant' }>; onChange: (allow: Capability[]) => void; onCancel: () => void; onSave: () => void; twin?: string }) {
  const m = screen.plugin.manifest;
  const toggle = (c: Capability, on: boolean) => onChange(m.capabilities.filter((x) => (x === c ? on : screen.allow.includes(x))));
  return <>
    <Dialog.Title id={titleId} className="m-0 text-base font-semibold normal-case tracking-normal text-ink">{t(screen.dev ? 'plugin-grant-title' : 'plugin-grant-title-hub', { name: m.name })}</Dialog.Title>
    <div className="flex flex-wrap items-center gap-2 text-xs text-muted">
      <PluginMark dev={screen.dev} />
      {!screen.dev && <span>{t('plugin-from-hub')}</span>}
      <span>{t('plugin-version-by', { version: m.version, author: m.author })}</span>
      <span>{t('plugin-licence', { licence: m.licence })}</span>
    </div>
    {twin !== undefined && <p className="m-0" role="note" data-plugin-replaces="">{t('plugin-replaces-hub', { name: twin })}</p>}
    <p className="m-0 text-muted">{t('plugin-grant-never')}</p>
    {m.capabilities.length === 0 ? <p className="m-0">{t('plugin-grant-nothing')}</p> : <fieldset className="m-0 grid gap-2 border-0 p-0" data-plugin-grants="">
      <legend className="mb-1 p-0 font-medium text-ink">{t('plugin-grant-asks')}</legend>
      {m.capabilities.map((c) => (
        <label key={c} className="touch-row flex items-start gap-2">
          <input type="checkbox" className="mt-0.5 size-4 flex-none" checked={screen.allow.includes(c)} onChange={(e) => toggle(c, e.target.checked)} data-cap={c} />
          <span>{t(capabilityKey(c))}</span>
        </label>
      ))}
    </fieldset>}
    <div className="mt-1 flex flex-wrap justify-end gap-2">
      <Button onClick={onCancel}>{t('plugin-cancel')}</Button>
      <Button variant="primary" onClick={onSave} data-plugin-save="">{t(screen.text !== undefined ? 'plugin-add' : screen.fresh ? 'plugin-show' : 'plugin-save')}</Button>
    </div>
  </>;
}

// The hub's community catalog, read only: what is not installed on the hub (or is newer there), what each asks to see, and the command
// the hub's owner runs on the hub's machine. The command is plain text to copy; nothing here installs.
function CatalogList({ entries, installed }: { entries: readonly CatalogPlugin[]; installed: (id: string) => string | undefined }) {
  const offered = entries.flatMap((e) => {
    const have = installed(e.id);
    if (have !== undefined && compareVersions(e.version, have) <= 0) return [];
    return [{ e, upgrade: have !== undefined }];
  });
  return <section className="grid gap-2" data-plugin-catalog="">
    <h3 className="m-0 text-sm font-semibold normal-case tracking-normal text-ink">{t('plugin-catalog-title')}</h3>
    <p className="m-0 text-xs text-muted">{t(offered.length ? 'plugin-catalog-intro' : 'plugin-catalog-none')}</p>
    {offered.length > 0 && <ul className="m-0 grid list-none gap-2 p-0">
      {offered.map(({ e, upgrade }) => {
        const command = `agora hub plugins ${upgrade ? 'upgrade' : 'install'} ${e.id}`;
        return (
          <li key={e.id} className="grid gap-1.5 rounded-m border border-line p-3" data-catalog-row={e.id}>
            <div className="flex min-w-0 flex-wrap items-center gap-2">
              <span className="min-w-0 truncate font-medium text-ink">{e.name}</span>
              <span className="text-xs text-muted">{t('plugin-version-by', { version: e.version, author: e.author })}</span>
              <span className="text-xs text-muted">{t('plugin-licence', { licence: e.licence })}</span>
            </div>
            {e.description && <p className="m-0 text-xs text-muted">{e.description}</p>}
            <p className="m-0 text-xs">{e.capabilities.length || e.unknown ? t('plugin-catalog-asks', { what: [...e.capabilities.map((c) => t(capabilityKey(c))), ...(e.unknown ? [t('plugin-catalog-unknown', { count: e.unknown })] : [])].join('; ') }) : t('plugin-grant-nothing')}</p>
            <p className="m-0 text-xs text-muted">{t(upgrade ? 'plugin-catalog-upgrade-how' : 'plugin-catalog-install-how')}</p>
            <code className="break-all font-mono text-caption text-muted select-all" data-catalog-command="">{command}</code>
          </li>
        );
      })}
    </ul>}
  </section>;
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
