import { useSyncExternalStore } from 'react';
import { type PluginState, pluginStore } from './store';

/** The plugins this browser loaded, as React state. */
export function usePlugins(): PluginState {
  return useSyncExternalStore(pluginStore.subscribe, pluginStore.state, pluginStore.state);
}
