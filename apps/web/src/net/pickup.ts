// ADR-0131 (Amendment A): automatic pickup in the owner's window. The owner turns it on per Review-enabled shared project, names the
// machines and agent kinds that may use it, and marks the open tasks an idle agent may take. The three commands are the owner's alone
// (unscoped); the marks and pickups reach the window as board facts (net/view.ts).
import type { agora } from '../gen/agora.js';
import { CommandError, type CommandBody } from './live';

type Command = (node: string, body: CommandBody) => Promise<agora.v1.CommandOk.$Properties>;

/** The settings as the hub says them: `inactiveReason` is `review_off`, `not_shared`, or a reason a newer hub may add. */
export interface PickupSettings { enabled: boolean; nodeIds: string[]; agentKinds: string[]; active: boolean; inactiveReason: string }

/** The hub's limits on the allow-lists: at most 32 entries, each 1 to 64 of `[A-Za-z0-9._-]`. */
export const LIST_MAX = 32;
export const validEntry = (s: string): boolean => /^[A-Za-z0-9._-]{1,64}$/.test(s);

function settingsOf(s: agora.v1.TaskPickupSettings.$Properties | null | undefined): PickupSettings {
  if (!s) throw new CommandError('protocol.invalid_message');
  return { enabled: !!s.enabled, nodeIds: [...(s.nodeIds ?? [])], agentKinds: [...(s.agentKinds ?? [])], active: !!s.active, inactiveReason: s.inactiveReason ?? '' };
}

export async function readPickup(command: Command, projectId: string): Promise<PickupSettings> {
  return settingsOf((await command('', { taskPickupSettingsGet: { projectId } })).taskPickupSettings);
}

/** Saves the settings (lists cut to what the hub takes) and reads them back, so the window shows what the hub keeps and why it is off. */
export async function savePickup(command: Command, projectId: string, next: Pick<PickupSettings, 'enabled' | 'nodeIds' | 'agentKinds'>): Promise<PickupSettings> {
  const list = (xs: string[]) => [...new Set(xs.filter(validEntry))].slice(0, LIST_MAX);
  await command('', { taskPickupSettingsSet: { projectId, enabled: next.enabled, nodeIds: list(next.nodeIds), agentKinds: list(next.agentKinds) } });
  return readPickup(command, projectId);
}

export async function markPickup(command: Command, projectId: string, taskId: string, eligible: boolean): Promise<void> {
  await command('', { taskPickupEligibleSet: { projectId, taskId, eligible } });
}
