// What a refused change of an agent says (ADR-0090). A command that did not go through carries a stable code; the window says it in plain
// words from its locale files, and falls back to its general sentence for a code it does not know.

/** `true`: it went through. `false`: it did not, for a reason not known. A string: the refusal's stable code. */
export type ActionResult = boolean | string;

const KEYS: Record<string, string> = {
  'agent.option_unconfirmed': 'refusal-unconfirmed',
  'agent.option_forbidden': 'refusal-forbidden',
  'agent.option_unknown': 'refusal-unknown-option',
  'agent.queue_full': 'prompt-queue-full',
  'protocol.capability_missing': 'refusal-unsupported',
};

/** The locale key to say for a refusal code; undefined for none known (the caller uses its own general sentence). */
export const refusalKey = (code: string): string | undefined => KEYS[code];

/** Every locale key `refusalKey` can return, for tests. */
export const REFUSAL_KEYS: readonly string[] = [...new Set(Object.values(KEYS))];
