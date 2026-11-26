import * as Dialog from '@radix-ui/react-dialog';
import { createContext, useContext, useEffect, useId, useLayoutEffect, useRef, useState, useSyncExternalStore } from 'react';
import { t } from '../i18n';
import type { AgentTileModel } from '../model';
import { closeAsk, closeReasonKey, losesWork, useCloseAsk, type CloseAsk } from '../net/closeAgent';
import { Menu, type MenuItem } from '../layout/Menu';
import { soundPrefs } from '../net/sounds';
import { canPlaySounds } from './chime';
import { useSoundPrefs } from './SoundSettings';
import { Icon } from './Icon';
import { Button } from './kit';

// What the window can do to end an agent: `AgentStop` for the agent it is given (undefined when it ended, else the error code). A context, so
// the console, the full-size view and the phone read it without passing it down. Absent without a hub (the gallery and the demo): no control.
export const CloseAgentContext = createContext<((agent: AgentTileModel) => Promise<string | undefined>) | undefined>(undefined);

/** Whether this agent can be closed from here: running, and the window can send the command. */
export const useCanClose = (agent: AgentTileModel): boolean => !!useContext(CloseAgentContext) && !agent.closed;

// The agent's menu: "Close agent" opens the confirm for the agent of that pane. Not "Stop turn" (that one is in the composer and keeps the
// agent). While sounds are on in this browser it also mutes this agent's sounds (remembered here; net/sounds.ts).
export function CloseAgentMenu({ agent, where = 'pane' }: { agent: AgentTileModel; where?: CloseAsk['where'] }) {
  const can = useCanClose(agent);
  const sounds = useSoundPrefs().on && canPlaySounds() && !agent.closed;
  const muted = useSyncExternalStore(soundPrefs.subscribe, () => soundPrefs.isMuted(agent.id), () => false);
  const items: MenuItem[] = [
    ...(sounds ? [{ label: t('agent-sounds-mute'), checked: muted, run: () => soundPrefs.setMuted(agent.id, !muted) }] : []),
    ...(can ? [{ label: t('close-agent'), divider: sounds, run: () => closeAsk.ask(agent.id, where) }] : []),
  ];
  if (items.length === 0) return null;
  return <Menu label={t('agent-actions', { name: agent.name })} items={items}><Icon name="more" /></Menu>;
}

// One dialog for one ask, even where the agent is drawn twice at once (two panes on the same agent): of the confirms drawn for the ask,
// the first one mounted shows it. Only confirms for the current ask register, so the list is only ever theirs.
const owners: string[] = [];
const ownerSubs = new Set<() => void>();
const ownersChanged = () => { for (const s of [...ownerSubs]) s(); };
const subscribeOwners = (fn: () => void) => { ownerSubs.add(fn); return () => { ownerSubs.delete(fn); }; };
const firstOwner = () => owners[0];
function useFirstToShow(wants: boolean): boolean {
  const me = useId();
  useLayoutEffect(() => {
    if (!wants) return;
    owners.push(me);
    ownersChanged();
    return () => { const at = owners.indexOf(me); if (at >= 0) owners.splice(at, 1); ownersChanged(); };
  }, [wants, me]);
  return useSyncExternalStore(subscribeOwners, firstOwner, firstOwner) === me;
}

// The confirm (audit B15, #1070): the kit's dialog look (a red icon chip, a title that names the agent, what happens, a warning when a
// turn is under way), then Keep it running first, with the focus, and the red Close agent last (ADR-0102). Escape, a click outside and
// Keep all leave the agent running; only the red button closes it. Built from kit parts here: the kit's ConfirmDialog (#1056) has no
// busy or error state. Drawn only where it was asked for, for the agent that was asked for.
export function CloseConfirm({ agent, where }: { agent: AgentTileModel; where: CloseAsk['where'] }) {
  const close = useContext(CloseAgentContext);
  const asked = useCloseAsk();
  const mine = asked?.id === agent.id && asked.where === where;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | undefined>();
  const content = useRef<HTMLDivElement>(null);
  const body = `close-body-${useId()}`;
  // Closed meanwhile (here, from another window, or after a permission was given): nothing is left to ask.
  useEffect(() => { if (agent.closed && asked?.id === agent.id) closeAsk.cancel(); }, [agent.closed, asked?.id, agent.id]);
  useEffect(() => { if (!mine) { setBusy(false); setError(undefined); } }, [mine]);
  const shows = useFirstToShow(mine && !!close && !agent.closed);
  if (!mine || !close || agent.closed || !shows) return null;
  const go = async () => {
    setBusy(true);
    setError(undefined);
    const code = await close(agent);
    setBusy(false);
    if (code) setError(code); else closeAsk.cancel();
  };
  return (
    <Dialog.Root open onOpenChange={(o) => { if (!o) closeAsk.cancel(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-40 bg-surface-0/70 backdrop-blur-sm" />
        {/* Its keys stay in it: Escape closes this dialog only (never the pane, the panel or the list it was opened from, whose React tree
            the portal shares), and a typed letter is never a window shortcut behind it. */}
        <Dialog.Content ref={content} role="alertdialog" aria-describedby={body} data-close-confirm onOpenAutoFocus={(e) => { e.preventDefault(); content.current?.querySelector<HTMLButtonElement>('[data-close-keep]')?.focus(); }}
          onKeyDown={(e) => { if (e.key === 'Escape' || (!e.ctrlKey && !e.metaKey && !e.altKey && e.key.length === 1)) e.stopPropagation(); }}
          className="fixed left-1/2 top-1/2 z-50 grid max-h-[calc(100dvh-2rem)] w-[min(28rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 grid-cols-[auto_minmax(0,1fr)] items-start gap-x-3 gap-y-1 overflow-auto rounded-l border border-line bg-surface-raised p-5 text-ink shadow-pop">
          <span aria-hidden className="kit-chip kit-tone-danger row-span-2"><Icon name="close" /></span>
          <Dialog.Title className="m-0 min-w-0 break-words text-title font-semibold normal-case tracking-normal text-ink">{t('close-title', { name: agent.name })}</Dialog.Title>
          <div id={body} className="col-start-2 m-0 min-w-0 text-muted">
            <p className="m-0">{t('close-body')}</p>
            {losesWork(agent) && <p className="m-0 mt-1 text-danger" data-close-warning>{t('close-working')}</p>}
            {error && <p role="alert" className="m-0 mt-1 text-danger">{t('close-failed-prefix', { reason: t(closeReasonKey(error)) })}</p>}
          </div>
          <div className="col-span-2 mt-4 flex flex-wrap justify-end gap-2">
            <Button data-close-keep onClick={() => closeAsk.cancel()}>{t('close-keep')}</Button>
            <Button variant="danger" disabled={busy} onClick={() => void go()}><Icon name="close" className="size-3.5" />{t('close-agent')}</Button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
