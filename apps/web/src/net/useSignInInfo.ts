// Whether the hub asks this browser for the screen password, and whether one is set (ADR-0067), as React state. It is read again
// whenever the window's session changes: setting, changing or clearing the password elsewhere (`agora password set|clear`, or another
// window) signs this window out, so a sign-in or sign-out is exactly when the answer may have changed (#909). Without that, Settings
// in a tab opened before the password was set offered "Set password" with no current-password field, and the hub refused it.
// A read that gets no answer (the hub unreachable for a moment) changes nothing: the last answer stands, so a set password is never
// shown as unset because of a network error (#960 review).
import { useCallback, useEffect, useRef, useState } from 'react';
import { NO_PASSWORD, readSignInInfo, type HubConnection, type SignInInfo } from './session';

/** The info, and `refresh`, which reads it again and resolves to the hub's answer (`undefined` when there was none). */
export function useSignInInfo(hub: HubConnection | undefined, fetcher: typeof fetch = fetch): [SignInInfo, () => Promise<SignInInfo | undefined>] {
  const [info, setInfo] = useState<SignInInfo>(NO_PASSWORD);
  // Only the latest read lands: an answer that arrives after a newer question is dropped.
  const asked = useRef(0);
  const refresh = useCallback(async () => {
    const n = ++asked.current;
    const answer = await readSignInInfo(fetcher);
    if (answer && n === asked.current) setInfo(answer);
    return answer;
  }, [fetcher]);
  const session = hub?.token;
  useEffect(() => { void refresh(); }, [refresh, session]);
  return [info, refresh];
}
