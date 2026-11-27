// The mark the window shows for an agent kind (visual-language.md "Agent identity"): a registry kind's own vendored icon (ADR-0092), or,
// for a built-in kind, the registry icon `manifests/agents/icons.json` names for it. Every icon is a plain one-colour drawing built into
// this window and is only ever used as a mask (`AgentAvatar`), never fetched from a machine and never rendered as markup. No icon: the
// avatar shows the kind's first letter.
import table from '../../../../manifests/agents/icons.json';
import { registryIcon, registryInfo } from './registryCatalog';

const ID = /^[a-z0-9_-]{1,64}$/;

/** The built-in kinds' lines, cleaned: a kind id and a registry icon id that the window has. Exported for the tests. */
export function builtInIcons(raw: unknown): ReadonlyMap<string, string> {
  const kinds = typeof raw === 'object' && raw !== null ? (raw as { kinds?: unknown }).kinds : undefined;
  const out = new Map<string, string>();
  if (typeof kinds !== 'object' || kinds === null) return out;
  for (const [kind, id] of Object.entries(kinds as Record<string, unknown>)) {
    if (!ID.test(kind) || typeof id !== 'string' || !ID.test(id)) continue;
    const url = registryIcon(id);
    if (url) out.set(kind, url);
  }
  return out;
}

const builtIn = builtInIcons(table);

/** The URL of a kind's icon, when the window has one. */
export const kindIcon = (kind: string | undefined): string | undefined =>
  kind ? (registryInfo(kind)?.icon ?? builtIn.get(kind)) : undefined;
