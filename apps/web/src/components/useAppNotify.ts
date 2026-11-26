import { useEffect, useSyncExternalStore } from 'react';
import { appNotify } from '../net/desktopNotify';

/** Inside a desktop app that has its own notifications: asked of the app once per page, then kept (net/desktopNotify.ts). */
export function useAppNotify(): boolean {
  useEffect(() => { appNotify.probe(); }, []);
  return useSyncExternalStore(appNotify.subscribe, appNotify.supported, () => false);
}
