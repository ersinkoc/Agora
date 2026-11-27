import { useCallback, useEffect, useId, useState } from 'react';
import { t } from '../i18n';
import { namesSecret, parseValues, publishErrorKey, refusalWords, validShareName, type ShareItemDef, type SharedItem, type SharedSource } from '../net/share';
import { Icon } from './Icon';
import { Button, Callout, ConfirmDialog, Pill, SegmentedControl } from './kit';
import { SettingsBlock, SettingsCard } from './SettingsCard';

/** A project a shared server can be limited to: its id (the same on every machine that has it) and how the window names it. */
export interface SharedProject { projectId: string; label: string }

type Runs = 'program' | 'address';
interface Draft { name: string; runs: Runs; command: string; args: string; env: string; url: string; headers: string; projectId: string }
const EMPTY: Draft = { name: '', runs: 'program', command: '', args: '', env: '', url: '', headers: '', projectId: '' };

/** The item a draft describes, or the locale key of what is wrong with it. */
export function draftItem(d: Draft): ShareItemDef | string {
  if (!validShareName(d.name)) return 'shared-check-name';
  if (d.runs === 'program') {
    const env = parseValues(d.env, '=');
    if (!d.command.trim()) return 'shared-check-command';
    if (!env) return 'shared-check-env';
    const args = d.args.split('\n').map((a) => a.trim()).filter(Boolean);
    if (args.some(namesSecret)) return 'shared-check-args-secret';
    return { name: d.name, kind: 'mcp_server', mcp: { command: d.command.trim(), args, env, url: '', headers: [] } };
  }
  const headers = parseValues(d.headers, ':');
  if (!/^https:\/\/\S+$/.test(d.url.trim())) return 'shared-check-url';
  if (!headers) return 'shared-check-headers';
  return { name: d.name, kind: 'mcp_server', mcp: { command: '', args: [], env: [], url: d.url.trim(), headers } };
}

const usesSecret = (d: Draft) => (d.runs === 'program' ? namesSecret(d.env) : namesSecret(d.headers));

// Settings → Shared (ADR-0122): the MCP servers the owner gives every agent at its start, on every machine that trusts the device that
// published them. Each publish and each removal is signed here, with this device's key; a machine checks the signature and its own
// policy, and what a machine refused is shown on the item in words. A server whose environment or headers name a secret is not given
// to agents yet (the machine's launcher that fills secrets is not built), and the form says so before it is published.
export function SharedSettings({ source, machineName, projects, canSign }: {
  source: SharedSource; machineName: (id: string) => string; projects: readonly SharedProject[];
  // Whether this browser has a device key to sign with; without one the list is shown and nothing can be published.
  canSign: boolean;
}) {
  const [items, setItems] = useState<SharedItem[] | 'failed' | undefined>();
  const [draft, setDraft] = useState<Draft | undefined>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | undefined>();
  const [removing, setRemoving] = useState<SharedItem | undefined>();
  const id = useId();
  const load = useCallback(() => source.list().then(setItems, () => setItems('failed')), [source]);
  useEffect(() => { void load(); }, [load]);
  const list = Array.isArray(items) ? items : [];
  const projectLabel = (p: string) => (p ? projects.find((x) => x.projectId === p)?.label ?? p : t('shared-scope-all'));

  const publish = async () => {
    if (!draft) return;
    const item = draftItem(draft);
    if (typeof item === 'string') { setError(item); return; }
    setBusy(true);
    const before = list.find((x) => x.name === item.name);
    const r = await source.publish(item, { projectId: draft.projectId, kinds: [] }, before?.issuedMs);
    setBusy(false);
    if (!r.ok) { setError(publishErrorKey(r)); return; }
    setDraft(undefined);
    setError(undefined);
    void load();
  };
  const remove = async () => {
    const item = removing;
    setRemoving(undefined);
    if (!item) return;
    setBusy(true);
    const r = await source.remove(item);
    setBusy(false);
    setError(r.ok ? undefined : publishErrorKey(r));
    void load();
  };
  const edit = (x: SharedItem) => {
    const m = x.mcp;
    setError(undefined);
    setDraft({
      name: x.name, runs: m?.url ? 'address' : 'program', command: m?.command ?? '', args: (m?.args ?? []).join('\n'),
      env: (m?.env ?? []).map((v) => `${v.name}=${v.value}`).join('\n'), url: m?.url ?? '',
      headers: (m?.headers ?? []).map((v) => `${v.name}: ${v.value}`).join('\n'), projectId: x.scope.projectId,
    });
  };
  const set = (patch: Partial<Draft>) => setDraft((d) => (d ? { ...d, ...patch } : d));
  const field = (name: string) => `${id}-${name}`;

  return (
    <SettingsCard id="shared" icon="puzzle" title={t('settings-shared')} description={t('shared-desc')}
      end={!draft && canSign ? <Button size="sm" onClick={() => { setError(undefined); setDraft(EMPTY); }}><Icon name="plus" className="size-3.5" />{t('shared-add')}</Button>
        : list.length > 0 ? <Pill>{t('shared-count', { count: list.length })}</Pill> : undefined}>
      {items === undefined ? <SettingsBlock><p className="muted m-0" role="status">{t('shared-loading')}</p></SettingsBlock>
        : items === 'failed' ? <SettingsBlock><p className="m-0" role="status">{t('shared-load-failed')}</p></SettingsBlock>
        : list.length === 0 && !draft ? <SettingsBlock><p className="muted m-0">{t(canSign ? 'shared-empty' : 'shared-empty-no-device')}</p></SettingsBlock>
        : list.length > 0 && (
          <ul className="set-list" aria-label={t('settings-shared')}>
            {list.map((x) => (
              <li key={x.name} className="set-item">
                <span className="set-os" aria-hidden><Icon name={x.mcp?.url ? 'globe' : 'terminal'} className="size-4" /></span>
                <span className="set-item__main">
                  <span className="set-item__name"><bdi>{x.name}</bdi></span>
                  <span className="set-item__meta">
                    <span className="font-mono">{x.mcp ? (x.mcp.url || [x.mcp.command, ...x.mcp.args].join(' ')) : t('shared-kind-skill')}</span>
                    <span>{projectLabel(x.scope.projectId)}</span>
                  </span>
                </span>
                <span className="set-item__end">
                  {x.refused.length === 0 ? <Pill tone="finished" icon={<Icon name="check" className="size-3" />}>{t('shared-no-refusals')}</Pill>
                    : <Pill tone="plain" icon={<Icon name="warn" className="size-3" />}>{t('shared-refused-count', { count: x.refused.length })}</Pill>}
                  {canSign && <Button size="sm" variant="ghost" disabled={busy} onClick={() => edit(x)}>{t('shared-change')}</Button>}
                  {canSign && <Button size="sm" variant="danger-ghost" disabled={busy} onClick={() => setRemoving(x)} aria-label={t('shared-remove-named', { name: x.name })}><Icon name="trash" className="size-3.5" /></Button>}
                </span>
                {x.refused.map((r) => {
                  const w = refusalWords(r);
                  const reason = t(w.key, w.args);
                  return <p key={r.node} className="set-item__note">{t('shared-refused-on', { machine: machineName(r.node), reason })}</p>;
                })}
              </li>
            ))}
          </ul>
        )}
      {draft && (
        <form className="set-form" aria-label={t('shared-form')} onSubmit={(e) => { e.preventDefault(); if (!busy) void publish(); }}>
          <div className="set-form__row">
            <label htmlFor={field('name')} className="set-form__label">{t('shared-name')}</label>
            <input id={field('name')} value={draft.name} onChange={(e) => set({ name: e.target.value.trim() })} placeholder="docs-search" autoComplete="off" spellCheck={false} maxLength={64} />
          </div>
          <div className="set-form__row">
            <span className="set-form__label" id={field('runs')}>{t('shared-runs')}</span>
            <SegmentedControl labelledBy={field('runs')} value={draft.runs} onChange={(runs) => set({ runs })}
              items={[{ value: 'program', label: t('shared-runs-program') }, { value: 'address', label: t('shared-runs-address') }]} />
          </div>
          {draft.runs === 'program' ? <>
            <div className="set-form__row">
              <label htmlFor={field('command')} className="set-form__label">{t('shared-command')}</label>
              <input id={field('command')} className="font-mono" value={draft.command} onChange={(e) => set({ command: e.target.value })} placeholder="npx" autoComplete="off" spellCheck={false} />
            </div>
            <div className="set-form__row">
              <label htmlFor={field('args')} className="set-form__label">{t('shared-args')}</label>
              <textarea id={field('args')} rows={3} value={draft.args} onChange={(e) => set({ args: e.target.value })} placeholder={t('shared-args-hint')} spellCheck={false} />
            </div>
            <div className="set-form__row">
              <label htmlFor={field('env')} className="set-form__label">{t('shared-env')}</label>
              <textarea id={field('env')} rows={2} value={draft.env} onChange={(e) => set({ env: e.target.value })} placeholder="NAME=value" spellCheck={false} />
            </div>
          </> : <>
            <div className="set-form__row">
              <label htmlFor={field('url')} className="set-form__label">{t('shared-url')}</label>
              <input id={field('url')} type="url" className="font-mono" value={draft.url} onChange={(e) => set({ url: e.target.value })} placeholder="https://" autoComplete="off" spellCheck={false} />
            </div>
            <div className="set-form__row">
              <label htmlFor={field('headers')} className="set-form__label">{t('shared-headers')}</label>
              <textarea id={field('headers')} rows={2} value={draft.headers} onChange={(e) => set({ headers: e.target.value })} placeholder="Name: value" spellCheck={false} />
            </div>
          </>}
          <div className="set-form__row">
            <label htmlFor={field('project')} className="set-form__label">{t('shared-scope')}</label>
            <select id={field('project')} value={draft.projectId} onChange={(e) => set({ projectId: e.target.value })}>
              <option value="">{t('shared-scope-all')}</option>
              {projects.map((p) => <option key={p.projectId} value={p.projectId}>{p.label}</option>)}
            </select>
          </div>
          {(draft.runs === 'address' || usesSecret(draft)) && (
            <SettingsBlock>
              {draft.runs === 'address' && <Callout tone="danger" icon="warn" title={t('shared-url-warning-title')}>{t('shared-url-warning')}</Callout>}
              {usesSecret(draft) && <Callout icon="key" title={t('shared-secret-title')}>{t('shared-secret')}</Callout>}
            </SettingsBlock>
          )}
          <div className="set-form__row">
            <span aria-hidden className="set-form__label" />
            <div className="set-form__actions">
              <Button type="submit" variant="primary" disabled={busy}>{t(list.some((x) => x.name === draft.name) ? 'shared-publish-change' : 'shared-publish')}</Button>
              <Button onClick={() => { setDraft(undefined); setError(undefined); }} disabled={busy}>{t('shared-cancel')}</Button>
            </div>
          </div>
        </form>
      )}
      {error && <SettingsBlock><p role="alert" className="m-0 text-danger">{t(error)}</p></SettingsBlock>}
      <ConfirmDialog open={!!removing} title={t('shared-remove-title', { name: removing?.name ?? '' })} body={t('shared-remove-body')}
        cancel={t('shared-remove-keep')} confirm={t('shared-remove-confirm')} onCancel={() => setRemoving(undefined)} onConfirm={() => void remove()} />
    </SettingsCard>
  );
}
