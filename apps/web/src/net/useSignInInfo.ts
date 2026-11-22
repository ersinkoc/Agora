// Whether the hub asks this browser for the screen password, and whether one is set (ADR-0067), as React state. It is read again
// whenever the window's session changes: setting, changing or clearing the password elsewhere (`agora password set|clear`, or another
// window) signs this window out, so a sign-in or sign-out is exactly when the answer may have changed (#909). Without that, Settings
// in a tab opened before the password was set offered "Set password" with no current-password field, and the hub refused it.
import { useCallback, useEffect, useRef, useState } from 'react';
import { signInInfo, type HubConnection, type SignInInfo } from './session';

export function useSignInInfo(hub: HubConnection | undefined, fetcher: typeof fetch = fetch): [SignInInfo, () => void, (i: SignInInfo) => void] {
  const [info, setInfo] = useState<SignInInfo>({ passwordRequired: false, passwordSet: false });
  // Only the latest read lands: an answer that arrives after a newer question is dropped.
  const asked = useRef(0);
  const refresh = useCallback(() => {
    const n = ++asked.current;
    void signInInfo(fetcher).then((i) => { if (n === asked.current) setInfo(i); });
  }, [fetcher]);
  const session = hub?.token;
  useEffect(() => { refresh(); }, [refresh, session]);
  return [info, refresh, setInfo];
}
