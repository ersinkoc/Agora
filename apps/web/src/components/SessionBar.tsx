import { useEffect, useMemo, useRef, useState } from 'react';
import { refusalKey, type ActionResult } from '../net/refusal';
import { NoAskBadge } from './NoAskBadge';
import { approvedHere, skipAnswerOf } from '../net/skipApproval';
import { settingName } from './launchSettingsModel';
import { startedWith } from './launchMemory';
import { Icon } from './Icon';
import { Pill, stateTone } from './kit';
import { Mark } from './StateBadge';
import { Menu, type MenuItem } from '../layout/Menu';
import { duration } from '../net/time';
import { t } from '../i18n';
import type { SessionModel, SessionOptionModel, UiState } from '../model';
import type { ConfigOptionModel } from '../model';
import { actsFreely, skippingNow, choices, choiceState, configChoices, configGroups, contextFill, contextLabel, contextPercent, isChange, needsPlaceholder, shortCount, type OptionKind } from './sessionChoice';

// ADR-0090: the word a setting is called in the window. The three the window knows get its own words (the agent's name for them is its
// own and untrusted); any other keeps the agent's name.
const CATEGORY_WORD: Record<string, string> = { mode: 'session-mode', model: 'session-model', thought_level: 'session-effort' };
const configWord = (o: ConfigOptionModel): string => (CATEGORY_WORD[o.category] ? t(CATEGORY_WORD[o.category] as string) : o.name || o.id);

// ADR-0058: the mode and model a chat agent runs with, how full its context is, and what it says it cost. The pickers appear only
// for what the agent reported. A mode or setting in which the agent acts without asking is sent like any other choice: the machine raises
// an approval card the owner signs (ADR-0090 Amendment A); until it is answered the control says it waits and shows no new value.
// `state` and `tokens` belong to the header (`status`): the agent's state as the kit's status pill with its running time, and the tokens used so
// far. This is the one place a chat agent's state and tokens are shown: the window's and the pane's headers leave them out for a chat agent.
// A switch the machine holds until the owner signs the card: which control, the value asked for and the card (`<node>/<approval id>`).
// `asking`: the card waits; `confirming`: this window approved it and the agent has not reported it yet; `late`: that took long, the control shows what the agent reports.
interface Wait { control: string; value: string; approval: string; state: 'asking' | 'confirming' | 'late'; at: number }
// The messages about a held switch that go away once the agent reports the value.
const HELD_NOTES = ['session-skip-approved-pending', 'session-skip-unconfirmed'];
const CONFIRM_MS = 20_000; // how long an approved switch may take to be reported before the window says so
const waitWord = (w: Wait): string => t(w.state === 'confirming' ? 'session-waiting-confirm' : 'session-waiting-approval');
const NOBODY_MS = 30_000; // how long a switch is held when the window has no view of the cards at all
const refusedKey = (control: string, value: string) => `${control}|${value}`;

export function SessionBar({ agentKey, session, choose, setConfig, disabled, startedMs, state, tokens, approvals, launchedFree, part = 'all' }: { part?: 'all' | 'controls' | 'status'; agentKey: string; session: SessionModel; startedMs?: number; state?: UiState; tokens?: number; choose?: (agentKey: string, kind: OptionKind, id: string) => Promise<ActionResult>; setConfig?: (agentKey: string, configId: string, value: string) => Promise<ActionResult>; disabled?: boolean; approvals?: ReadonlySet<string>; launchedFree?: boolean }) {
  const [waits, setWaits] = useState<Wait[]>([]);
  // Choices this machine's own settings refuse (`agent.option_forbidden`): kept marked unavailable, with the reason, for this agent.
  const [refused, setRefused] = useState<ReadonlySet<string>>(() => new Set());
  const seen = useRef(new Set<string>());
  const [failed, setFailed] = useState<string | false>(false); // the locale key of what to say when a change did not go through
  const [busy, setBusy] = useState(false);
  // How long it has run, when this window saw it start; moves once a second-ish (every 15 s is enough for minutes).
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => { if (!startedMs || disabled) return; const id = setInterval(() => setNow(Date.now()), 30000); return () => clearInterval(id); }, [startedMs, disabled]);
  const elapsed = startedMs && !disabled ? Math.max(0, now - startedMs) : undefined;
  const pct = contextPercent(session);
  const label = contextLabel(session);
  const said = label ? t(label.key, { percent: label.percent }) : '';
  // What the machine answered to a choice: applied, refused (by its own policy or otherwise), or held until the owner signs the card.
  const settle = (r: ActionResult, control: string, value: string) => {
    if (r === true) { setFailed(false); return; }
    if (typeof r === 'object') { setFailed(false); setWaits((w) => [...w.filter((x) => x.control !== control), { control, value, approval: r.approvalNeeded, state: 'asking' as const, at: Date.now() }]); return; }
    const code = typeof r === 'string' ? r : '';
    if (code === 'agent.option_forbidden') setRefused((c) => new Set(c).add(refusedKey(control, value)));
    setFailed(refusalKey(code) ?? 'session-change-failed');
  };
  // What the agent reports now for a control: a setting of its own, else the older mode or model.
  const reported = (s: SessionModel, control: string): string | undefined => {
    const c = s.config?.find((o) => o.id === control);
    if (c) return c.current;
    return control === 'mode' ? s.currentMode : control === 'model' ? s.currentModel : undefined;
  };
  const sessionNow = useRef(session);
  sessionNow.current = session;
  // A held switch ends when the agent reports the value (applied). When its card is gone without that: a denial put the control back at once; an
  // approval made here waits for the agent's report (said plainly if it takes long, and cleared when it comes); an outcome this window did not see
  // (answered elsewhere, ran out, withdrawn) gets a short grace, then the control says it was not switched. With no view of the cards at all the wait is bounded.
  useEffect(() => {
    if (waits.length === 0) return;
    const timers: ReturnType<typeof setTimeout>[] = [];
    const done = waits.filter((w) => disabled || reported(session, w.control) === w.value);
    const change = new Map<Wait, Wait | null>();
    for (const w of waits) {
      if (done.includes(w)) continue;
      const later = (ms: number, f: () => void) => timers.push(setTimeout(() => { if (reported(sessionNow.current, w.control) !== w.value) f(); }, ms));
      if (!approvals) { if (w.state !== 'late') later(NOBODY_MS, () => { setWaits((cur) => cur.filter((x) => x !== w)); setFailed('session-skip-unconfirmed'); }); continue; }
      if (approvals.has(w.approval)) { seen.current.add(w.approval); if (w.state !== 'asking') change.set(w, { ...w, state: 'asking' }); continue; }
      const outcome = skipAnswerOf(w.approval);
      if (outcome === 'approved') {
        if (w.state === 'asking') change.set(w, { ...w, state: 'confirming', at: Date.now() });
        else if (w.state === 'confirming') later(Math.max(0, w.at + CONFIRM_MS - Date.now()), () => { setWaits((cur) => cur.map((x) => (x === w ? { ...w, state: 'late' } : x))); setFailed('session-skip-approved-pending'); });
      } else later(outcome === 'denied' ? 300 : seen.current.has(w.approval) ? 2000 : 6000, () => { setWaits((cur) => cur.filter((x) => x !== w)); setFailed('session-skip-not-applied'); });
    }
    if (done.length > 0 || change.size > 0) {
      setWaits((cur) => cur.filter((x) => !done.includes(x)).map((x) => change.get(x) ?? x));
      // The agent reported it after the window had said it was still waiting: that note no longer holds.
      if (done.length > 0) setFailed((f) => (f && HELD_NOTES.includes(f) ? false : f));
    }
    return () => timers.forEach(clearTimeout);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [waits, approvals, session, disabled]);
  const apply = async (kind: OptionKind, id: string) => {
    if (!choose) return;
    setWaits((w) => w.filter((x) => x.control !== kind));
    setBusy(true);
    const r = await choose(agentKey, kind, id);
    setBusy(false);
    settle(r, kind, id);
  };
  const pick = (kind: OptionKind, id: string) => {
    if (!isChange(kind === 'mode' ? session.currentMode : session.currentModel, id)) return; // the placeholder and the one in use are not choices
    setFailed(false);
    void apply(kind, id);
  };
  // A custom menu, not a native select: it looks the same on every platform and in both themes, and lists only what the agent reported.
  const picker = (kind: OptionKind, list: readonly SessionOptionModel[], current: string) => {
    if (list.length === 0) return null;
    const word = t(kind === 'mode' ? 'session-mode' : 'session-model');
    const name = (o: SessionOptionModel) => (o.skipsPermissions ? t('session-mode-unasked', { name: o.name || o.id }) : (o.name || o.id));
    const now = list.find((o) => o.id === current);
    const wait = waits.find((x) => x.control === kind);
    const shown = wait && wait.state !== 'late' ? waitWord(wait) : now ? name(now) : needsPlaceholder(current) ? t('session-not-reported') : current;
    // A choice this machine's settings refuse stays marked unavailable with the reason; any other is sent, a skipping one waits for the owner's card.
    const items: MenuItem[] = choices(list, current).map((o) => { const no = refused.has(refusedKey(kind, o.id)); return { label: name(o), checked: o.id === current, disabled: disabled || busy || !choose || no, hint: no ? t('session-forbidden-hint') : undefined, run: () => pick(kind, o.id) }; });
    return (
      <Menu label={`${word}: ${shown}`} items={items} align="start" className="ide-chip h-7 max-w-56 min-w-0 cursor-pointer rounded-s px-2.5 hover:border-accent hover:text-ink">
        <span className="flex-none">{word}</span><span className="min-w-0 truncate font-medium text-ink">{shown}</span><Icon name="chevronDown" className="size-3" />
      </Menu>
    );
  };
  // ADR-0090: the agent's own settings. Choosing one sends `AgentSetConfig`; one the node marks as acting without asking is sent the same
  // way and waits for the owner's approval card (Amendment A).
  const groups = useMemo(() => configGroups(session), [session]);
  const applyConfig = async (o: ConfigOptionModel, value: string) => {
    if (!setConfig || value === o.current) return;
    setFailed(false);
    setWaits((w) => w.filter((x) => x.control !== o.id));
    setBusy(true);
    const r = await setConfig(agentKey, o.id, value);
    setBusy(false);
    settle(r, o.id, value);
  };
  const choiceName = (o: ConfigOptionModel, c: { id: string; name: string; skipsPermissions: boolean }): string => {
    const base = o.type === 'boolean' ? t(c.id === 'true' ? 'session-on' : 'session-off') : c.name || c.id;
    return c.skipsPermissions ? t('session-mode-unasked', { name: base }) : base;
  };
  const configItems = (o: ConfigOptionModel): MenuItem[] => configChoices(o).map((c) => {
    const st = choiceState(o, c.id);
    const no = refused.has(refusedKey(o.id, c.id));
    return { label: choiceName(o, c), checked: st === 'current', disabled: disabled || busy || !setConfig || no, hint: no ? t('session-forbidden-hint') : undefined, run: () => void applyConfig(o, c.id) };
  });
  const configShown = (o: ConfigOptionModel): string => {
    const w = waits.find((x) => x.control === o.id);
    if (w && w.state !== 'late') return waitWord(w);
    const now = configChoices(o).find((c) => c.id === o.current);
    return now ? choiceName(o, now) : o.current || t('session-not-reported');
  };
  const configChip = (o: ConfigOptionModel) => (
    <Menu key={o.id} label={`${configWord(o)}: ${configShown(o)}`} items={configItems(o)} align="start" className="ide-chip h-7 max-w-56 min-w-0 cursor-pointer rounded-s px-2.5 hover:border-accent hover:text-ink">
      <span className="flex-none">{configWord(o)}</span><span className="min-w-0 truncate font-medium text-ink">{configShown(o)}</span><Icon name="chevronDown" className="size-3" />
    </Menu>
  );
  const moreItems: MenuItem[] = groups.more.map((o) => ({ label: `${configWord(o)}: ${configShown(o)}`, items: configItems(o) }));
  // What this device chose when it started the agent (an effort, say): the agent does not report it, so it is shown read-only and only
  // when known. A model, mode or effort that the agent offers has its own menu.
  const hasEffort = groups.chips.some((o) => o.category === 'thought_level');
  const started = useMemo(() => startedWith(agentKey).filter(([id]) => !(id === 'model' && (session.models.length > 0 || groups.replaces.model)) && !(id === 'mode' && (session.modes.length > 0 || groups.replaces.mode)) && !(id === 'effort' && hasEffort)), [agentKey, session.models.length, session.modes.length, groups, hasEffort]);
  // What makes it act without asking now, in the agent's own words ("Mode: YOLO"), for the sign's tooltip.
  const freeCause = skippingNow(session).map(({ option, value }) => {
    if (option === 'mode') return `${t('session-mode')}: ${session.modes.find((o) => o.id === value)?.name || value}`;
    const o = (session.config ?? []).find((c) => c.id === option);
    if (!o) return value;
    const shown = o.type === 'boolean' ? t(value === 'true' ? 'session-on' : 'session-off') : configChoices(o).find((c) => c.id === value)?.name || value;
    return `${configWord(o)}: ${shown}`;
  }).join(', ');
  // The acts-without-asking sign (ADR-0059, ADR-0090 Amendment A) stands beside Mode, the control that usually causes it, as a red-tinted tag:
  // the bolt, no words, its meaning in its tooltip and name. Started so (`launchedFree`): for its whole life; switched so: while the agent's
  // current mode or setting skips. Without a Mode control it leads the row.
  const free = launchedFree ? <NoAskBadge key="free" agent={{ skipsPermissions: true }} look="tag" />
    : !disabled && actsFreely(session) ? <NoAskBadge key="free" agent={{ skipsPermissions: true }} session look="tag" cause={freeCause} approved={skippingNow(session).every((x) => approvedHere(agentKey, x.option, x.value))} />
    : null;
  const modePicker = !groups.replaces.mode ? picker('mode', session.modes, session.currentMode) : null;
  const modeChip = groups.chips.some((o) => o.category === 'mode');
  const controls = <>
    {!modePicker && !modeChip && free}
    {modePicker}{modePicker && free}
    {!groups.replaces.model && picker('model', session.models, session.currentModel)}
    {groups.chips.map((o) => (o.category === 'mode' ? [configChip(o), free] : configChip(o)))}
    {moreItems.length > 0 && <Menu label={t('session-more')} items={moreItems} align="start" className="ide-chip h-7 cursor-pointer rounded-s px-2.5 hover:border-accent hover:text-ink"><span>{t('session-more')}</span><Icon name="chevronDown" className="size-3" /></Menu>}
    {started.map(([id, value]) =><span key={id} className="ide-chip h-7 rounded-s px-2.5" title={t('session-started-hint')}>{t('session-started-with', { name: settingName({ id, label: `setting-${id}`, values: [], defaultValue: '' }, t), value })}</span>)}
    {waits.some((w) => w.state === 'asking') && <span role="status" className="text-xs text-needs-you">{t('session-waiting-approval-note')}</span>}
    {failed && <span role="status" className="text-xs text-danger">{t(failed)}</span>}
  </>;
  const word = state ? t(`state-${state}`) : '';
  const status = <>
    {state && (
      <span className="inline-flex" title={elapsed !== undefined ? t('session-elapsed-hint') : undefined} data-state-pill={state}>
        <Pill tone={stateTone(state)} icon={<Mark state={state} />}>
          {elapsed !== undefined && elapsed >= 60000 ? t('chat-state-since', { state: word, time: elapsed < 3_600_000 ? `${Math.floor(elapsed / 60000)} min` : duration(Math.floor(elapsed / 60000) * 60000) }) : word}
        </Pill>
      </span>
    )}
    {!state && elapsed !== undefined && <span className="ide-chip" title={t('session-elapsed-hint')}><Icon name="clock" className="size-3" />{duration(elapsed)}</span>}
    {state && <span className="flex-1" />}
    {pct !== undefined && (
      <span className="flex items-center gap-2 text-xs text-muted" title={t('session-context-detail', { used: shortCount(session.contextTokens), size: shortCount(session.contextWindow) })}>
        <span>{said}</span>
        {/* Drawn with design tokens, not a native <meter>: Chromium paints its empty track light grey, which reads as full in dark mode. */}
        <span role="meter" aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct} aria-label={said}
          className="relative h-1.5 w-16 overflow-hidden rounded-full border border-line bg-surface-2">
          <span className={`absolute inset-y-0 start-0 rounded-full ${contextFill(pct)}`} style={{ width: `${session.contextTokens > 0 ? Math.max(pct, 2) : 0}%` }} />
        </span>
      </span>
    )}
    {tokens ? <span className="text-xs text-muted tabular-nums">{t('usage-total-tokens', { count: tokens, short: shortCount(tokens) })}</span> : null}
  </>;
  // The controls (mode, model) belong with the composer and the status (context, time) with the header; `all` keeps them in one row.
  if (part === 'controls') return <div className="flex min-w-0 flex-col gap-1.5"><div className="flex flex-wrap items-center gap-x-2 gap-y-1.5" role="group" aria-label={t('session-header')}>{controls}</div></div>;
  if (part === 'status') return pct === undefined && elapsed === undefined && !state && !tokens ? null : <div className="flex w-full flex-wrap items-center gap-x-3 gap-y-1.5">{status}</div>;
  return (
    <div className="flex flex-none flex-col gap-1.5 border-b border-line pb-2">
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1.5" role="group" aria-label={t('session-header')}>{controls}{status}</div>
    </div>
  );
}
