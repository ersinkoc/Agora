import { t } from '../i18n';
import type { PaletteItem } from '../components/CommandPalette';
import type { AgentTileModel } from '../model';
import { visible } from './hidden';

// The route of the machines page, and of one machine on it.
export const machinesHash = (id?: string) => (id ? `#/machines/${encodeURIComponent(id)}` : '#/machines');
export const isMachinesHash = (hash: string) => hash === '#/machines' || hash.startsWith('#/machines/');
// Settings keeps its old address (#/device); #/settings is the same page. A section is one more segment: #/device/passkeys.
export const isSettingsHash = (hash: string) => ['#/device', '#/settings'].some((r) => hash === r || hash.startsWith(`${r}/`));
export const settingsSectionOf = (hash: string): string | undefined => (isSettingsHash(hash) ? hash.split('/')[2] || undefined : undefined);
export const machineOfHash = (hash: string): string | undefined => (hash.startsWith('#/machines/') ? decodeHash(hash.slice('#/machines/'.length)) || undefined : undefined);

// The projects the fleet has agents in, once each, by name.
export function projectsOf(agents: readonly AgentTileModel[]): { id: string; name: string }[] {
  const out = new Map<string, string>();
  for (const a of agents) if (a.projectId && !out.has(a.projectId)) out.set(a.projectId, a.project || a.projectId);
  return [...out].map(([id, name]) => ({ id, name }));
}

// A part of the address hash decoded for the window to use. The hash is untrusted input (a link from anywhere), and a
// malformed escape makes `decodeURIComponent` throw, which would take the whole page down while rendering: malformed
// parts decode to `undefined` instead (the phone's `nav.ts` reads hashes the same way).
export const decodeHash = (part: string): string | undefined => {
  try {
    return decodeURIComponent(part);
  } catch {
    return undefined;
  }
};

export interface PaletteExtras {
  machines: readonly { id: string; name: string }[];
  projects: readonly { id: string; name: string }[];
  /** Signed in to a hub (the demo window has no session to lock and no machines to allow). */
  hub: boolean;
  /** A phone has no project list to jump in. */
  phone: boolean;
  /** A screen password is set: only then does Lock mean something (without one, anyone can sign in again). */
  passwordSet: boolean;
  /** This session may open the window for new machines. */
  canPair: boolean;
  go: (hash: string) => void;
  showProject: (id: string) => void;
  showPairing: () => void;
  lock?: () => void;
}

// Jump targets and a few actions. Entries are names and navigation only: nothing here carries a code, token or key, and none answers a
// request or opens the window for new machines by itself: "Allow a new machine to join" shows where that is done, and that screen asks.
export function paletteExtras(x: PaletteExtras): PaletteItem[] {
  return [
    ...x.machines.map((m) => ({ id: `machine-${m.id}`, label: t('palette-open-machine', { name: visible(m.name) }), run: () => x.go(machinesHash(m.id)), group: 'places' as const, icon: 'machine' as const })),
    ...(x.phone ? [] : x.projects.map((p) => ({ id: `project-${p.id}`, label: t('palette-open-project', { name: visible(p.name) }), run: () => x.showProject(p.id), group: 'places' as const, icon: 'folder' as const }))),
    ...(x.hub && x.canPair ? [{ id: 'pair', label: t('pair-window-allow'), run: x.showPairing, icon: 'plus' as const }] : []),
    ...(x.hub && x.passwordSet && x.lock ? [{ id: 'lock', label: t('lock-now'), run: x.lock, icon: 'lock' as const }] : []),
    { id: 'settings', label: t('palette-settings'), run: () => x.go('#/device'), group: 'go', icon: 'settings' },
  ];
}
