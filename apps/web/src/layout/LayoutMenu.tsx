import * as Dialog from '@radix-ui/react-dialog';
import { useState, type ReactNode } from 'react';
import { t } from '../i18n';
import { Icon } from '../components/Icon';
import { type Action, type Preset, type Workspace, PRESETS } from './engine';
import { Menu, type MenuItem } from './Menu';
import { MAX_NAME, type Named, deleteNamed, renameNamed, saveNamed, savedLayouts } from './store';
import { type LayoutError, MAX_DEPTH, MAX_PANES, deserialiseLayout, serialiseLayout } from './tree';

export const errorText = (e: LayoutError) => t(`layout-error-${e}`, { max: e === 'too-deep' ? MAX_DEPTH : MAX_PANES });

// The layout menu in the title bar: presets, maximize/restore, saved layouts, export/import, reset.
export function LayoutMenu({ ws, dispatch, agents, onReset, narrow }: {
  ws: Workspace;
  dispatch: (a: Action) => void;
  agents: () => string[]; // what a preset fills its panes with, in order
  onReset: () => void;
  narrow: boolean;
}) {
  const [dialog, setDialog] = useState<'save' | 'list' | 'import' | undefined>();
  const items = (): MenuItem[] => [
    ...PRESETS.map((p: Preset) => ({ label: t(`layout-preset-${p}`), disabled: narrow && p !== '1x1', run: () => dispatch({ type: 'preset', preset: p, agents: agents() }) })),
    ws.zoom ? { label: t('layout-restore'), divider: true, run: () => dispatch({ type: 'restore' }) }
      : { label: t('layout-maximize'), divider: true, disabled: !ws.focus, run: () => ws.focus && dispatch({ type: 'maximize', pane: ws.focus }) },
    { label: t('layout-save-as'), divider: true, run: () => setDialog('save') },
    { label: t('layout-saved'), run: () => setDialog('list') },
    { label: t('layout-export'), run: () => download(serialiseLayout(ws.layout)) },
    { label: t('layout-import'), run: () => setDialog('import') },
    { label: t('layout-reset'), divider: true, run: onReset },
  ];
  return (
    <>
      <Menu label={t('layout-menu')} items={items} className="ide-iconbtn"><Icon name="layout" /></Menu>
      {dialog === 'save' && <SaveDialog onSave={(name) => { saveNamed(name, ws.layout); setDialog(undefined); }} onClose={() => setDialog(undefined)} />}
      {dialog === 'list' && <ListDialog onApply={(l) => { dispatch({ type: 'replace', layout: l.layout }); setDialog(undefined); }} onClose={() => setDialog(undefined)} />}
      {dialog === 'import' && <ImportDialog onImport={(layout) => { dispatch({ type: 'replace', layout }); setDialog(undefined); }} onClose={() => setDialog(undefined)} />}
    </>
  );
}

function download(text: string) {
  try {
    const url = URL.createObjectURL(new Blob([text], { type: 'application/json' }));
    const a = document.createElement('a');
    a.href = url;
    a.download = 'layout.json';
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  } catch { /* nothing to save to here */ }
}

function Shell({ title, onClose, children }: { title: string; onClose: () => void; children: ReactNode }) {
  return (
    <Dialog.Root open onOpenChange={(open) => { if (!open) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-40 bg-surface-0/70 backdrop-blur-sm" />
        <Dialog.Content aria-describedby={undefined} className="fixed left-1/2 top-1/2 z-50 grid max-h-[85vh] w-[min(30rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 gap-3 overflow-y-auto rounded-l border border-line bg-surface-1 p-5 shadow-card">
          <Dialog.Title className="m-0 text-base font-semibold tracking-normal text-ink normal-case">{title}</Dialog.Title>
          {children}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function SaveDialog({ onSave, onClose }: { onSave: (name: string) => void; onClose: () => void }) {
  const [name, setName] = useState('');
  const taken = savedLayouts().some((x) => x.name === name.trim());
  return (
    <Shell title={t('layout-save-title')} onClose={onClose}>
      <form className="grid gap-3" onSubmit={(e) => { e.preventDefault(); if (name.trim()) onSave(name); }}>
        <label className="grid gap-1 text-[13px]">{t('layout-save-name')}
          <input autoFocus value={name} maxLength={MAX_NAME} onChange={(e) => setName(e.target.value)} />
        </label>
        {taken && <p className="muted m-0">{t('layout-save-replaces')}</p>}
        <div className="flex justify-end gap-2">
          <button type="button" className="button" onClick={onClose}>{t('layout-cancel')}</button>
          <button type="submit" className="button button--primary" disabled={!name.trim()}>{t('layout-save')}</button>
        </div>
      </form>
    </Shell>
  );
}

function ListDialog({ onApply, onClose }: { onApply: (l: Named) => void; onClose: () => void }) {
  const [list, setList] = useState(savedLayouts);
  const [renaming, setRenaming] = useState<{ from: string; to: string } | undefined>();
  return (
    <Shell title={t('layout-list-title')} onClose={onClose}>
      {list.length === 0 ? <p className="muted m-0">{t('layout-list-empty')}</p> : (
        <ul className="m-0 grid list-none gap-1 p-0">
          {list.map((l) => (
            <li key={l.name} className="flex flex-wrap items-center gap-2 rounded-xs border border-line bg-surface-0 px-2.5 py-1.5">
              {renaming?.from === l.name ? (
                <form className="flex min-w-0 flex-1 gap-2" onSubmit={(e) => { e.preventDefault(); setList(renameNamed(l.name, renaming.to)); setRenaming(undefined); }}>
                  <input autoFocus aria-label={t('layout-save-name')} className="min-w-0 flex-1 py-1" value={renaming.to} maxLength={MAX_NAME} onChange={(e) => setRenaming({ from: l.name, to: e.target.value })} />
                  <button type="submit" className="button py-1" disabled={!renaming.to.trim()}>{t('layout-save')}</button>
                </form>
              ) : <span className="min-w-0 flex-1 truncate font-medium">{l.name}</span>}
              {renaming?.from !== l.name && <>
                <button type="button" className="button button--primary py-1" onClick={() => onApply(l)}>{t('layout-apply')}</button>
                <button type="button" className="button py-1" onClick={() => setRenaming({ from: l.name, to: l.name })}>{t('layout-rename')}</button>
                <button type="button" className="button py-1" onClick={() => setList(deleteNamed(l.name))} aria-label={t('layout-delete-named', { name: l.name })}>{t('layout-delete')}</button>
              </>}
            </li>
          ))}
        </ul>
      )}
      <div className="flex justify-end"><button type="button" className="button" onClick={onClose}>{t('layout-close')}</button></div>
    </Shell>
  );
}

function ImportDialog({ onImport, onClose }: { onImport: (l: import('./tree').Layout) => void; onClose: () => void }) {
  const [text, setText] = useState('');
  const [error, setError] = useState<LayoutError | undefined>();
  const submit = () => {
    const parsed = deserialiseLayout(text);
    if (parsed.ok) onImport(parsed.layout); else setError(parsed.error);
  };
  return (
    <Shell title={t('layout-import-title')} onClose={onClose}>
      <label className="grid gap-1 text-[13px]">{t('layout-import-paste')}
        <textarea className="min-h-40 rounded-s border border-line bg-surface-0 p-2 font-mono text-xs text-ink" value={text} spellCheck={false}
          aria-invalid={!!error} aria-describedby={error ? 'layout-import-error' : undefined} onChange={(e) => { setText(e.target.value); setError(undefined); }} />
      </label>
      <label className="grid gap-1 text-[13px]">{t('layout-import-file')}
        <input type="file" accept=".json,application/json" className="text-xs" onChange={(e) => {
          const f = e.target.files?.[0];
          if (f) void f.text().then((s) => { setText(s); setError(undefined); }).catch(() => setError('json'));
        }} />
      </label>
      {error && <p id="layout-import-error" role="alert" className="m-0 text-needs-you">{errorText(error)}</p>}
      <div className="flex justify-end gap-2">
        <button type="button" className="button" onClick={onClose}>{t('layout-cancel')}</button>
        <button type="button" className="button button--primary" disabled={!text.trim()} onClick={submit}>{t('layout-import-do')}</button>
      </div>
    </Shell>
  );
}
