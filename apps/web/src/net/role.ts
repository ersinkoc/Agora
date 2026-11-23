// ADR-0106: the role of the person this window signed in as, from the `Welcome` the hub sends first on every connection. A hint for what
// the window shows and asks, never a permission: the hub checks every command. `unspecified` is an older hub (it sends no `Welcome`),
// where the window keeps learning the role from the hub's answer to `PairStatus`, as before.
import { agora } from '../gen/agora.js';
import type { PairOwner } from './pairing';

export type PersonRole = 'unspecified' | 'owner' | 'member' | 'viewer';

const ROLES: Record<number, PersonRole> = {
  [agora.v1.PersonRole.PERSON_ROLE_OWNER]: 'owner',
  [agora.v1.PersonRole.PERSON_ROLE_MEMBER]: 'member',
  [agora.v1.PersonRole.PERSON_ROLE_VIEWER]: 'viewer',
};

/** The role a `Welcome` names; `unspecified` for one that names none (a node's) or that cannot be read. */
export function welcomeRole(payload: Uint8Array): PersonRole {
  try {
    return ROLES[agora.v1.Welcome.decode(payload).personRole ?? 0] ?? 'unspecified';
  } catch {
    return 'unspecified';
  }
}

/**
 * What the role already says about the pairing cards, so the window need not ask `PairStatus`: a member or viewer is not the owner
 * (the hub would refuse them and write the refusal down). `undefined`: ask, as the owner (to learn whether pairing is on) and an older
 * hub do.
 */
export const ownerOfRole = (role: PersonRole): PairOwner | undefined => (role === 'member' || role === 'viewer' ? 'no' : undefined);

/** Whether the window offers actions that change something (start, stop, resume, remove, typing, prompts). A viewer only watches. */
export const mayChange = (role: PersonRole): boolean => role !== 'viewer';
