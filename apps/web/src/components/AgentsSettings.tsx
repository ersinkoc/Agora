import { useEffect, useMemo, useState } from 'react';
import { t } from '../i18n';
import type { MachineModel } from '../net/view';
import { ago } from '../net/time';
import { groupKinds, isRegistryKind, kindAccent, kindChoices, plainText, readKnown, runsAs, writeKnown, type KindInfo, type KnownKinds } from '../net/agentKinds';
import { kindIcon } from '../net/kindIcons';
import { registryInfo, type RegistryInfo } from '../net/registryCatalog';
import { Icon } from './Icon';
import { AgentAvatar, KeyValue, Pill, SegmentedControl, Tag, Tooltip } from './kit';
import { labelOf } from './launchChoice';
import { settingName } from './launchSettingsModel';
import { SettingsBlock, SettingsCard } from './SettingsCard';
import type { KindChoice } from './StartAgent';

const storage = (): Storage | undefined => { try { return window.localStorage; } catch { return undefined; } };
const toInfo = (k: KindChoice, level: number): KindInfo => ({ kind: k.kind, name: k.name, installed: k.installed, level, program: k.program ?? '', settings: k.settings ?? [], options: k.options ?? [], provider: k.provider ?? '' });
// More machines than this are chosen from a list: a segmented control would not fit them.
const SEGMENTS_MAX = 4;

// Settings → Agents: for one machine at a time, the agents it can start as a grid of cards, installed first, then the ones it cannot find
// (dashed). A card says how the agent runs and what it takes; a bolt marks one that offers to act without asking. Opening a card shows the
// rest: the values each setting accepts, its options (the one that skips permission questions labelled), and the program a machine misses.
// Read only: the choices are made when an agent starts (nothing here is saved as a default, and no choice that skips permission
// questions is ever preselected). A machine that is offline shows what it reported last, marked as older.
// ADR-0092: the agents the machine has are listed first; the ones it cannot find follow under "Add an agent", with what the window knows of
// an ACP registry agent (version, licence, repository) and its "Untested" mark. Looking at the list runs nothing and fetches nothing.
export function AgentsSettings({ machines, loadKinds }: { machines: readonly MachineModel[]; loadKinds: (node: string) => Promise<(KindChoice & { level?: number })[]> }) {
  const [known, setKnown] = useState<Map<string, KnownKinds>>(() => readKnown(storage()));
  const [failed, setFailed] = useState<ReadonlySet<string>>(new Set());
  const [chosen, setChosen] = useState<string | undefined>();
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
  const machine = machines.find((m) => m.id === chosen) ?? machines[0];
  const name = (m: MachineModel) => plainText(m.name, 24);
  const picker = machines.length < 2 ? (machine ? <Tag>{name(machine)}</Tag> : undefined)
    : machines.length <= SEGMENTS_MAX ? <SegmentedControl label={t('agents-machine')} value={machine?.id ?? ''} onChange={setChosen} items={machines.map((m) => ({ value: m.id, label: name(m) }))} />
    : (
      <select aria-label={t('agents-machine')} value={machine?.id ?? ''} onChange={(e) => setChosen(e.target.value)}>
        {machines.map((m) => <option key={m.id} value={m.id}>{name(m)}</option>)}
      </select>
    );
  return (
    <SettingsCard icon="terminal" title={t('settings-agents')} description={t('agents-desc')} end={picker}>
      {machine ? <MachineAgents key={machine.id} machine={machine} known={known.get(machine.id)} failed={failed.has(machine.id)} now={Date.now()} />
        : <SettingsBlock><p className="muted m-0">{t('machines-empty')}</p></SettingsBlock>}
    </SettingsCard>
  );
}

function MachineAgents({ machine, known, failed, now }: { machine: MachineModel; known: KnownKinds | undefined; failed: boolean; now: number }) {
  const groups = useMemo(() => groupKinds(known?.kinds ?? []), [known]);
  const stale = !machine.online || failed;
  const { installed, missing } = groups;
  return (
    <div className="set-kinds-wrap" aria-label={plainText(machine.name)} role="group">
      {stale && known && <p className="m-0"><Pill tone="idle">{t('agents-stale', { when: ago(known.atMs, now) })}</Pill></p>}
      {!known ? <p className="muted m-0" role="status">{t(stale ? 'agents-none-known' : 'agents-loading')}</p>
        : installed.length + missing.length === 0 ? <p className="muted m-0">{t('agents-empty')}</p>
        : <>
          {installed.length > 0 && <ul className="set-kinds" aria-label={t('agents-group-here')}>{installed.map((k) => <KindCard key={k.kind} k={k} />)}</ul>}
          {missing.length > 0 && <section className="set-kinds-add" aria-labelledby={`add-${machine.id}`}>
            <div className="set-kinds-add__head">
              <h4 id={`add-${machine.id}`} className="set-kinds-add__title">{t('agents-group-add')} <span className="set-kinds-add__count">{missing.length}</span></h4>
              <p className="muted m-0">{t('agents-group-add-desc')}</p>
            </div>
            <ul className="set-kinds">{missing.map((k) => <KindCard key={k.kind} k={k} />)}</ul>
          </section>}
        </>}
    </div>
  );
}

/** The link text of a repository or site: its address without the scheme, shortened. */
const linkText = (href: string) => plainText(href.replace(/^https:\/\//, '').replace(/\/$/, ''), 40);

/** What the window knows of an ACP registry agent, as key–value rows. */
function registryFacts(reg: RegistryInfo) {
  const link = (href: string) => <a href={href} target="_blank" rel="noopener noreferrer" className="set-kind__link">{linkText(href)}</a>;
  return [
    { label: t('agents-version'), value: reg.version, mono: true },
    // ADR-0092 open question 3: the licence's own text is a link, never bundled.
    ...(reg.license ? [{ label: t('agents-license'), value: reg.licenseUrl ? <a href={reg.licenseUrl} target="_blank" rel="noopener noreferrer" className="set-kind__link">{reg.license}</a> : reg.license }] : []),
    ...(reg.repository ? [{ label: t('agents-repository'), value: link(reg.repository) }] : []),
    ...(!reg.repository && reg.website ? [{ label: t('agents-website'), value: link(reg.website) }] : []),
    { label: t('agents-source'), value: t('agents-source-registry') },
  ];
}

function KindCard({ k }: { k: KindInfo }) {
  const { settings, options } = kindChoices(k);
  const untested = isRegistryKind(k.kind);
  const reg = registryInfo(k.kind);

  const how = runsAs(k.level);
  const name = plainText(k.name || k.kind);
  const skips = options.some((o) => o.skipsPermissions);
  const caption = [
    ...(k.installed ? [t(`agents-runs-short-${how}`), ...settings.map((s) => plainText(settingName(s, t), 24))] : [t('agents-not-installed')]),
    // ADR-0092: the mark of an ACP registry kind, on the card's face, in words.
    ...(untested ? [t('agents-untested')] : []),
  ].join(' · ');
  const facts = [
    { label: t('agents-kind'), value: plainText(k.kind, 32), mono: true },
    { label: t('agents-runs'), value: t(`agents-runs-${how}`) },
    ...settings.map((s) => ({
      label: plainText(settingName(s, t), 24),
      value: `${s.values.length > 0 ? s.values.slice(0, 8).map((v) => plainText(v, 24)).join(', ') : t('agents-setting-any')}${s.defaultValue ? ` · ${t('agents-setting-default', { value: plainText(s.defaultValue, 24) })}` : ''}`,
    })),
    ...(options.length > 0 ? [{
      label: t('agents-options'),
      value: <span className="flex min-w-0 flex-wrap gap-x-3 gap-y-0.5">{options.map((o) => <span key={o.id} className={o.skipsPermissions ? 'text-danger' : undefined}>{plainText(labelOf(o, t), 32)}{o.skipsPermissions ? ` (${t('agents-option-skips')})` : ''}</span>)}</span>,
    }] : []),
    ...(reg ? registryFacts(reg) : []),
  ];
  return (
    <li className={`set-kind${k.installed ? '' : ' set-kind--missing'}`}>
      <details>
        <summary className="set-kind__face">
          <AgentAvatar accent={kindAccent(k.kind)} kind={name} icon={kindIcon(k.kind)} />
          <span className="set-kind__name"><span className="set-kind__title">{name}</span><span className="set-kind__caption">{caption}</span></span>
          {skips && (
            <Tooltip text={t('agents-offers-skip')}>
              <span role="img" aria-label={t('agents-offers-skip')} className="set-risk"><Icon name="bolt" className="size-3.5" /></span>
            </Tooltip>
          )}
          <Icon name="chevron" className="set-kind__chev size-3.5" />
        </summary>
        <div className="set-kind__more">
          <KeyValue items={facts} />
          {!k.installed && k.program && <p className="muted m-0">{t('agents-program-missing', { program: plainText(k.program, 32) })}</p>}
          {untested && <p className="muted m-0">{t('agents-untested-desc')}</p>}
          {reg && reg.runs !== 'binary' && <p className="muted m-0">{t('agents-runs-fetch', { program: reg.runs })}</p>}
        </div>
      </details>
    </li>
  );
}
