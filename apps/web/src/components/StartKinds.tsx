import { useId, useRef, type KeyboardEvent } from 'react';
import { t } from '../i18n';
import { isRegistryKind, runsAs } from '../net/agentKinds';
import { registryInfo } from '../net/registryCatalog';
import { AgentAvatar, Tag } from './kit';
import type { KindChoice } from './StartAgent';

// The colour of an agent is picked when it starts (from the id its machine gives it), so nothing here wears one: a neutral avatar.
export const NO_COLOUR = 'var(--text-muted)';

/** The kinds as the start window lists them, each group in the machine's own order: the installed hand-made ones, then the installed
 * ACP registry ones (ADR-0092, untested in Agora), then the hand-made ones that are not installed. A registry kind that is not installed is
 * left out here: there are dozens, and Settings → Agents lists them under "Add an agent". */
export function kindOrder(kinds: readonly KindChoice[]): { installed: KindChoice[]; untested: KindChoice[]; missing: KindChoice[] } {
  return {
    installed: kinds.filter((k) => k.installed && !isRegistryKind(k.kind)),
    untested: kinds.filter((k) => k.installed && isRegistryKind(k.kind)),
    missing: kinds.filter((k) => !k.installed && !isRegistryKind(k.kind)),
  };
}

// "Agent" in the start window: one card per kind (its letter, its name, Terminal or Chat), a radio group with one tab stop. The arrow keys
// move and choose (mirrored in a right-to-left page), Home and End go to the ends. A kind the machine cannot find is still a card, dashed
// and under "Not installed" (said to assistive tech in its description): choosing it shows why it cannot start, and the window does not
// start it. A card's name is the kind's name
// alone (its tag and "not installed" are its description), so a person and a script find it by that name.
export function KindCards({ kinds, value, onChange, disabled, label }: {
  kinds: readonly KindChoice[]; value: string; onChange: (kind: string) => void; disabled?: boolean; label: string;
}) {
  const id = useId();
  const { installed, untested, missing } = kindOrder(kinds);
  const all = [...installed, ...untested, ...missing];
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const at = all.findIndex((k) => k.kind === value);
  const stop = at >= 0 ? at : 0;
  const move = (to: number) => {
    const n = (to + all.length) % all.length;
    const k = all[n];
    if (!k) return;
    onChange(k.kind);
    refs.current[n]?.focus();
  };
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const rtl = getComputedStyle(e.currentTarget).direction === 'rtl';
    const step = { ArrowRight: rtl ? -1 : 1, ArrowLeft: rtl ? 1 : -1, ArrowDown: 1, ArrowUp: -1 }[e.key];
    if (step !== undefined) move((at >= 0 ? at : step > 0 ? -1 : 0) + step);
    else if (e.key === 'Home') move(0);
    else if (e.key === 'End') move(all.length - 1);
    else return;
    e.preventDefault();
  };
  const card = (k: KindChoice, i: number) => {
    const on = k.kind === value;
    const how = runsAs(k.level ?? 0) === 'terminal' ? 'start-runs-terminal' : 'start-runs-chat';
    return (
      <button key={k.kind} ref={(el) => { refs.current[i] = el; }} type="button" role="radio" aria-checked={on} tabIndex={i === stop ? 0 : -1}
        aria-label={k.name} aria-describedby={`${id}-${i}`} disabled={disabled} data-kind={k.kind} data-selected={on ? '' : undefined}
        className={`kit-card kit-card--dense flex min-w-0 items-center gap-2.5 disabled:opacity-50 ${k.installed ? '' : 'border-dashed'}`}
        onClick={() => onChange(k.kind)}>
        <AgentAvatar accent={NO_COLOUR} kind={k.name || k.kind} icon={registryInfo(k.kind)?.icon} />
        <span className="grid min-w-0 gap-0.5">
          <span className={`truncate font-medium ${k.installed ? 'text-ink' : 'text-muted'}`}>{k.name}</span>
          <span id={`${id}-${i}`} className="flex flex-wrap gap-1"><Tag>{t(how)}</Tag>{isRegistryKind(k.kind) && <Tag>{t('agents-untested')}</Tag>}{!k.installed && <span className="sr-only">{t('start-not-installed')}</span>}</span>
        </span>
      </button>
    );
  };
  const grid = 'grid grid-cols-[repeat(auto-fill,minmax(9rem,1fr))] gap-2';
  const group = 'm-0 text-caption font-medium text-muted';
  return (
    <div role="radiogroup" aria-label={label} className="grid gap-2" onKeyDown={onKey}>
      {missing.length + untested.length > 0 && installed.length > 0 && <p aria-hidden className={group}>{t('start-group-installed')}</p>}
      {installed.length > 0 && <div className={grid}>{installed.map((k, i) => card(k, i))}</div>}
      {untested.length > 0 && <p aria-hidden className={group}>{t('start-group-untested')}</p>}
      {untested.length > 0 && <div className={grid}>{untested.map((k, i) => card(k, installed.length + i))}</div>}
      {missing.length > 0 && <p aria-hidden className={group}>{t('start-group-missing')}</p>}
      {missing.length > 0 && <div className={grid}>{missing.map((k, i) => card(k, installed.length + untested.length + i))}</div>}
    </div>
  );
}
