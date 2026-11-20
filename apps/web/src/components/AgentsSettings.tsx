import { useEffect, useMemo, useState } from 'react';
import { t } from '../i18n';
import type { MachineModel } from '../net/view';
import { ago } from '../net/time';
import { groupKinds, kindChoices, plainText, readKnown, runsAs, writeKnown, type KindInfo, type KnownKinds } from '../net/agentKinds';
import { labelOf } from './launchChoice';
import { settingName } from './launchSettings';
import type { KindChoice } from './StartAgent';

const storage = (): Storage | undefined => { try { return window.localStorage; } catch { return undefined; } };
const toInfo = (k: KindChoice, level: number): KindInfo => ({ kind: k.kind, name: k.name, installed: k.installed, level, program: k.program ?? '', settings: k.settings ?? [], options: k.options ?? [] });

// Settings → Agents: for each machine, the agents it can start, installed first, with how each runs and what it takes when started.
// Read only: the choices are made when an agent starts (nothing here is saved as a default, and no choice that skips permission
// questions is ever preselected). A machine that is offline shows what it reported last, marked as older.
export function AgentsSettings({ machines, loadKinds }: { machines: readonly MachineModel[]; loadKinds: (node: string) => Promise<(KindChoice & { level?: number })[]> }) {
  const [known, setKnown] = useState<Map<string, KnownKinds>>(() => readKnown(storage()));
  const [failed, setFailed] = useState<ReadonlySet<string>>(new Set());
  const online = machines.filter((m) => m.online).map((m) => m.id).join('\n');
  useEffect(() => {
    let live = true;
    for (const node of online.split('\n').filter(Boolean)) {
      loadKinds(node).then((list) => {
        if (!live) return;
        setFailed((f) => { const n = new Set(f); n.delete(node); return n; });
        setKnown((prev) => { const next = new Map(prev); next.set(node, { atMs: Date.now(), kinds: list.map((k) => toInfo(k, k.level ?? 0)) }); writeKnown(storage(), next); return next; });
      }, () => { if (live) setFailed((f) => new Set(f).add(node)); });
    }
    return () => { live = false; };
  }, [online, loadKinds]);
  const now = Date.now();
  return (
    <>
      <p className="muted m-0">{t('agents-intro')}</p>
      {machines.length === 0 && <p className="muted m-0">{t('machines-empty')}</p>}
      {machines.map((m) => <MachineAgents key={m.id} machine={m} known={known.get(m.id)} failed={failed.has(m.id)} now={now} />)}
    </>
  );
}

function MachineAgents({ machine, known, failed, now }: { machine: MachineModel; known: KnownKinds | undefined; failed: boolean; now: number }) {
  const groups = useMemo(() => groupKinds(known?.kinds ?? []), [known]);
  const stale = !machine.online || failed;
  return (
    <section className="grid gap-2" aria-label={plainText(machine.name)}>
      <h3 className="m-0 flex flex-wrap items-baseline gap-x-2"><span className="min-w-0 truncate">{plainText(machine.name)}</span>
        {stale && known && <span className="muted text-xs font-normal">{t('agents-stale', { when: ago(known.atMs, now) })}</span>}</h3>
      {!known ? <p className="muted m-0" role="status">{t(stale ? 'agents-none-known' : 'agents-loading')}</p>
        : groups.installed.length + groups.missing.length === 0 ? <p className="muted m-0">{t('agents-empty')}</p>
        : <>
          <KindList title={t('agents-installed')} kinds={groups.installed} stale={stale} />
          <KindList title={t('agents-not-installed')} kinds={groups.missing} stale={stale} />
        </>}
    </section>
  );
}

function KindList({ title, kinds, stale }: { title: string; kinds: KindInfo[]; stale: boolean }) {
  if (kinds.length === 0) return null;
  return (
    <div className="grid gap-1">
      <h4 className="m-0 text-xs font-semibold uppercase tracking-wide text-muted">{title} · {kinds.length}</h4>
      <ul className="m-0 grid list-none gap-2 p-0">{kinds.map((k) => <KindRow key={k.kind} k={k} stale={stale} />)}</ul>
    </div>
  );
}

function KindRow({ k, stale }: { k: KindInfo; stale: boolean }) {
  const { settings, options } = kindChoices(k);
  const how = runsAs(k.level);
  return (
    <li className={`grid gap-1 rounded-xs border border-line bg-surface-1 px-3 py-2 ${stale ? 'opacity-80' : ''}`}>
      <div className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0.5">
        <strong className="min-w-0 max-w-full truncate">{plainText(k.name || k.kind)}</strong>
        <span className="mono muted max-w-full truncate text-xs">{plainText(k.kind, 32)}</span>
        <span className="muted text-xs">{t(`agents-runs-${how}`)}</span>
        <span className={`text-xs ${k.installed ? 'text-finished' : 'muted'}`}>{t(k.installed ? 'agents-state-installed' : 'agents-state-missing')}</span>
      </div>
      {!k.installed && k.program && <p className="muted m-0 text-xs">{t('agents-program-missing', { program: plainText(k.program, 32) })}</p>}
      {(settings.length > 0 || options.length > 0) && (
        <dl className="m-0 grid gap-x-3 gap-y-0.5 text-xs sm:grid-cols-[max-content_minmax(0,1fr)]">
          {settings.map((s) => (
            <div key={`s-${s.id}`} className="contents">
              <dt className="muted">{plainText(settingName(s, t), 24)}</dt>
              <dd className="m-0 min-w-0 break-words">{s.values.length > 0 ? s.values.slice(0, 8).map((v) => plainText(v, 24)).join(', ') : t('agents-setting-any')}{s.defaultValue ? ` · ${t('agents-setting-default', { value: plainText(s.defaultValue, 24) })}` : ''}</dd>
            </div>
          ))}
          {options.length > 0 && (
            <div className="contents">
              <dt className="muted">{t('agents-options')}</dt>
              <dd className="m-0 flex min-w-0 flex-wrap gap-x-3 gap-y-0.5">
                {options.map((o) => <span key={o.id} className={o.skipsPermissions ? 'text-needs-you' : undefined}>{plainText(labelOf(o, t), 32)}{o.skipsPermissions ? ` (${t('agents-option-skips')})` : ''}</span>)}
              </dd>
            </div>
          )}
        </dl>
      )}
    </li>
  );
}
