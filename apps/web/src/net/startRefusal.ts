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
  'auth.forbidden': 'start-directory-forbidden',
};

/** The locale key for a refused start's code. */
export const startRefusalKey = (code: string | undefined): string =>
  code !== undefined && Object.prototype.hasOwnProperty.call(KEYS, code) ? KEYS[code] as string : 'start-failed';
