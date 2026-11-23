// The owner's answer to a machine that wants a grant before it runs a command (ADR-0041): the question shown, Allow (sign, send the grant,
// then send the refused command again) and Not now. While Allow is signing and sending, the question cannot be dismissed, and the command is
// sent again only if this is still the question being answered, so the window never says "nothing was sent" for a command that then goes.
import { useCallback, useEffect, useRef, useState } from 'react';
import type { agora } from '../gen/agora.js';
import { declineGive, isGive, resendGive } from './give';
import { grantFor, type GrantAsk } from './grant';
import type { CommandBody } from './live';

export interface GrantQuestion { id: number; node: string; body: CommandBody; ask: GrantAsk; busy: boolean; failed: boolean }

type Command = (node: string, body: CommandBody) => Promise<agora.v1.CommandOk.$Properties>;
/** `sign` makes the grant (`signGrant` with this device's key); undefined when the window cannot sign (no hub link, no device key). */
export interface GrantDeps { command?: Command; sign?: (node: string, ask: GrantAsk) => Promise<agora.v1.GrantCreate.$Properties> }

export function useGrantFlow(deps: GrantDeps) {
  const [question, setQuestion] = useState<GrantQuestion | undefined>();
  const seq = useRef(0);
  // The question being answered now (its id), kept outside React's state so a continuation that resumes later can read it.
  const live = useRef<number | undefined>(undefined);
  const busy = useRef(false);
  const latest = useRef(deps);
  latest.current = deps;
  useEffect(() => { live.current = question?.id; }, [question?.id]);

  /** A machine refused a command until the owner allows it: ask, unless another question is open. */
  const request = useCallback((node: string, body: CommandBody) => {
    const ask = grantFor(body as Record<string, unknown>);
    if (!ask) return;
    setQuestion((cur) => cur ?? { id: ++seq.current, node, body, ask, busy: false, failed: false });
  }, []);

  const allow = useCallback(async () => {
    const g = question;
    const { command, sign } = latest.current;
    if (!g || busy.current) return;
    if (!command || !sign) { setQuestion((c) => c && { ...c, failed: true }); return; }
    busy.current = true;
    setQuestion({ ...g, busy: true, failed: false });
    try {
      const grant = await sign(g.node, g.ask);
      await command(g.node, { grantCreate: grant });
      busy.current = false;
      // Belt and braces: dismissal is refused while busy, but only the question still being answered sends its command again.
      if (live.current !== g.id) return;
      live.current = undefined;
      setQuestion(undefined);
      // A task given to an agent: the answer goes to its dialog (sent, queued or the refusal).
      if (isGive(g.body)) await resendGive(command, g.node, g.body);
      // A resize is sent again with the next one, and a refused subscription is tried again on its own stream by the window: neither is
      // resent here (a subscription on the commands stream is refused).
      else if (!('terminalResize' in g.body) && !('terminalSubscribe' in g.body) && !('chatSubscribe' in g.body)) await command(g.node, g.body).catch(() => {});
    } catch {
      busy.current = false;
      if (live.current === g.id) setQuestion({ ...g, busy: false, failed: true });
    }
  }, [question]);

  /** Not now (or Escape, or a click outside). Ignored while Allow is under way: that answer is already being sent. */
  const dismiss = useCallback(() => {
    const g = question;
    if (!g || busy.current || g.busy) return;
    live.current = undefined;
    if (isGive(g.body)) declineGive(g.node, g.body);
    setQuestion(undefined);
  }, [question]);

  return { question, request, allow, dismiss };
}
