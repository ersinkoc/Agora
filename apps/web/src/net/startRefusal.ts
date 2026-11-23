// What the start window says when a start was refused for a reason without its own handling in App's `launchAgent`: the locale key for
// the refusal's stable code, and the general "did not start" sentence for any other code.

const KEYS: Record<string, string> = {
  'agent.mode_unsupported': 'start-directory-agent-unsupported',
  'approval.signature_invalid': 'approval-not-trusted',
  'protocol.capability_missing': 'start-directory-unsupported',
  'agent.kind_unknown': 'start-kind-unknown',
  // #903: the machine cannot find the kind's program (the same check that marks it "not installed").
  'agent.program_missing': 'start-program-missing',
  'node.resource_limit': 'start-limit',
  // A machine that limits this device to other projects (ADR-0049 T1c); the hub's refusal for the person's role is `ROLE_REFUSAL`.
  'auth.forbidden': 'start-directory-forbidden',
  role: 'start-role',
};

/** The window's own reason (never a protocol code: those all hold a dot) for a command the hub refused because of the person's role or
 * projects (ADR-0049): a viewer, or a member outside their projects. Places that keep only a code word it for the person, not a device. */
export const ROLE_REFUSAL = 'role';

/**
 * A refusal's code as the window keeps it, for `command` (its schema name, `AgentLaunch`). The hub refuses a person's command with
 * `auth.forbidden` naming that schema name in `action`; a machine that limits this device to other projects names its own action
 * (`directory`, `grant.create`, `approval.resolve`). So `auth.forbidden` naming `command` becomes `ROLE_REFUSAL`, and anything else
 * keeps its code (#1044).
 */
export function refusalCode(e: unknown, command: string): string | undefined {
  if (typeof e !== 'object' || e === null) return undefined;
  const { code, params } = e as { code?: string; params?: Record<string, string> };
  return code === 'auth.forbidden' && params?.action === command ? ROLE_REFUSAL : code;
}

/** The locale key for a refused start's code. */
export const startRefusalKey = (code: string | undefined): string =>
  code !== undefined && Object.prototype.hasOwnProperty.call(KEYS, code) ? KEYS[code] as string : 'start-failed';
