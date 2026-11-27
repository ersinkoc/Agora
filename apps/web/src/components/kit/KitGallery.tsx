import { useState, type ReactNode } from 'react';
import { t } from '../../i18n';
import { AGENT_COLOUR_NAMES } from '../../themes/tokens';
import { Icon } from '../Icon';
import { registryInfo } from '../../net/registryCatalog';
import { AgentAvatar, Button, Callout, Card, CopyChip, EmptyState, KeyValue, ListRow, Pill, SegmentedControl, SettingRow, StatTile, StatusMark, StatusPill, Tag, Toggle, Tooltip, type ButtonVariant } from '.';

// The widget kit in every state, for the gallery (one per theme panel). Demo data only; nothing here is sent anywhere.
const VARIANTS: ButtonVariant[] = ['primary', 'secondary', 'ghost', 'danger', 'danger-ghost'];
const KINDS = ['claude-code', 'codex', 'gemini', 'wrongstack', 'opencode', 'aider', 'goose'];
const GROUPS = ['machine', 'project', 'kind', 'state'] as const;
type Group = (typeof GROUPS)[number];

function Section({ title, children }: { title: string; children: ReactNode }) {
  return <div className="kit-gallery__group"><h3 className="m-0 text-caption font-semibold text-muted">{title}</h3>{children}</div>;
}
const Row = ({ children }: { children: ReactNode }) => <div className="kit-gallery__row">{children}</div>;

function Groups({ by, onBy }: { by: Group; onBy: (g: Group) => void }) {
  return <SegmentedControl label={t('fleet-group-by')} items={GROUPS.map((g) => ({ value: g, label: t(`fleet-group-${g}`) }))} value={by} onChange={onBy} />;
}

export function KitGallery() {
  const [by, setBy] = useState<Group>('machine');
  const [notify, setNotify] = useState(true);
  const [free, setFree] = useState(true);
  const [pressed, setPressed] = useState(true);
  return (
    <section className="kit-gallery" data-kit aria-label={t('kit-title')}>
      <h2 className="kit-gallery__title">{t('kit-title')}</h2>
      <Section title={t('kit-buttons')}>
        {(['md', 'sm'] as const).map((size) => (
          <Row key={size}>{VARIANTS.map((v) => <Button key={v} variant={v} size={size}>{t(v.startsWith('danger') ? (v === 'danger' ? 'kit-sample-forget' : 'device-remove') : v === 'primary' ? 'action-start-agent' : v === 'ghost' ? 'fleet-filter-clear' : 'action-open')}</Button>)}
            <Button size={size} disabled>{t('action-open')}</Button></Row>
        ))}
      </Section>
      <Section title={t('kit-states')}>
        <Row>
          {(['working', 'needs-you', 'finished', 'idle', 'unknown'] as const).map((s) => <StatusPill key={s} state={s} />)}
          <StatusPill state="finished" failed />
        </Row>
        <Row><Pill tone="finished">{t('machine-online')}</Pill><Pill>{t('machine-offline')}</Pill><Pill tone="accent">{t('kit-sample-shared')}</Pill><Tag>Linux</Tag><Tag mono>0.3.34</Tag><Tag mono>fix/login</Tag></Row>
      </Section>
      <Section title={t('kit-avatars')}>
        <Row>
          {AGENT_COLOUR_NAMES.map((c, i) => <AgentAvatar key={c} accent={`var(--agent-${c})`} kind={KINDS[i] ?? c} />)}
          <AgentAvatar accent="var(--agent-amber)" kind="claude" size="s" />
          <AgentAvatar accent="var(--agent-teal)" kind="codex" size="l" needsYou label="codex · teal" />
          <AgentAvatar accent="var(--agent-slate)" kind="goose" icon={registryInfo('registry-goose')?.icon} label="goose" />
        </Row>
      </Section>
      <Section title={t('kit-cards')}>
        <div className="grid gap-3 sm:grid-cols-3">
          <Card><strong>laptop</strong><span className="text-muted">Linux · 0.3.34</span></Card>
          <Card selected onClick={() => {}}><strong>vps-1</strong><span className="text-muted">Linux · 0.3.34</span></Card>
          <Card density="dense"><KeyValue items={[{ label: t('device-name'), value: 'web-10a3e055', mono: true }, { label: t('device-fingerprint'), value: '91c2 9a46 6de1', mono: true }]} /></Card>
        </div>
        <Card density="flush">
          <div className="grid gap-0.5 p-2">
            <ListRow leading={<AgentAvatar accent="var(--agent-amber)" kind="claude" size="s" />} label="claude · amber" meta={<StatusMark state="needs-you" />} onClick={() => {}} />
            <ListRow current leading={<AgentAvatar accent="var(--agent-teal)" kind="codex" size="s" />} label="codex · teal" meta={<><Tag mono>±6</Tag><StatusMark state="working" /></>} onClick={() => {}} />
            <ListRow leading={<Icon name="folder" className="size-4 text-muted" />} label="login-fix" meta={<StatusMark state="finished" failed />} />
          </div>
          <SettingRow label={t('kit-sample-notify')} hint={t('kit-sample-notify-hint')}>{(l, h) => <Toggle checked={notify} onChange={setNotify} labelledBy={l} describedBy={h} />}</SettingRow>
          <SettingRow label={t('fleet-group-by')}>{(l) => <SegmentedControl labelledBy={l} items={GROUPS.map((g) => ({ value: g, label: t(`fleet-group-${g}`) }))} value={by} onChange={setBy} />}</SettingRow>
          <SettingRow label={t('agent-does-not-ask')} hint={t('agent-does-not-ask-explain')}>{(l, h) => <Toggle risk checked={free} onChange={setFree} labelledBy={l} describedBy={h} />}</SettingRow>
        </Card>
      </Section>
      <Section title={t('kit-controls')}>
        <Row>
          <Groups by={by} onBy={setBy} />
          <Toggle checked={false} onChange={() => {}} label={t('kit-sample-notify')} />
          <Toggle checked onChange={() => {}} label={t('kit-sample-notify')} />
          <Toggle risk checked onChange={() => {}} label={t('agent-does-not-ask')} />
          <Toggle checked disabled onChange={() => {}} label={t('kit-sample-notify')} />
          <Tooltip text={t('agent-address-copy')}><button type="button" className="ide-iconbtn" aria-label={t('agent-address-copy')}><Icon name="copy" /></button></Tooltip>
        </Row>
      </Section>
      <Section title={t('kit-counts')}>
        <Row>
          <StatTile value={2} tone="needs" label={t('state-needs-you')} mark={<StatusMark state="needs-you" />} />
          <StatTile value={3} label={t('state-working')} mark={<StatusMark state="working" />} pressed={pressed} onPress={() => setPressed(!pressed)} />
          <StatTile value={0} tone="needs" label={t('state-needs-you')} mark={<StatusMark state="needs-you" />} />
          <StatTile value={1} label={t('fleet-failed')} mark={<StatusMark state="finished" failed />} />
        </Row>
      </Section>
      <Section title={t('kit-notices')}>
        <Callout tone="needs" icon="terminal" title={t('kit-sample-ask', { name: 'codex · teal' })} actions={<><Button size="sm">{t('approval-deny')}</Button><Button size="sm" variant="primary">{t('approval-allow')}</Button></>}>
          <code className="font-mono">cargo test -p agora-core</code>
        </Callout>
        <Callout tone="info" icon="machine" title={t('kit-sample-offline')}>{t('kit-sample-offline-line')}</Callout>
        <Callout tone="danger" icon="warn" title={t('approval-failed')} actions={<Button size="sm" variant="ghost">{t('fleet-filter-clear')}</Button>} />
        <Callout tone="success" icon="check" title={t('kit-sample-joined')} />
      </Section>
      <Section title={t('kit-empty')}>
        <div className="grid gap-3 sm:grid-cols-2">
          <Card><EmptyState icon="chat" title={t('empty-messages-title')} line={t('empty-messages-hint')} action={<Button variant="primary" size="sm">{t('action-start-agent')}</Button>} /></Card>
          <Card><EmptyState ok icon="check" title={t('empty-changes-title')} line={t('empty-changes-hint')} /></Card>
        </div>
      </Section>
      <Section title={t('kit-copy')}>
        <div className="grid max-w-md gap-2"><CopyChip text="agora hub login" /></div>
      </Section>
      <Section title={t('kit-rtl')}>
        <div dir="rtl" className="grid gap-2">
          <Row><Groups by={by} onBy={setBy} /><Toggle checked={notify} onChange={setNotify} label={t('kit-sample-notify')} /><StatusPill state="working" /><AgentAvatar accent="var(--agent-violet)" kind="gemini" needsYou /></Row>
          <Callout tone="needs" icon="terminal" title={t('kit-sample-ask', { name: 'claude · amber' })} actions={<Button size="sm" variant="primary">{t('approval-allow')}</Button>} />
        </div>
      </Section>
    </section>
  );
}
