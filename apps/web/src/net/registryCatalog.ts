// ADR-0092: what the window knows about the ACP registry's agents. `manifests/agents/registry/` is generated from the registry and
// reviewed in a pull request; its `catalog.json` and vendored icons are built into this window, so nothing here is fetched from the
// registry (or anywhere else) when the list is shown. A machine reports its kinds (`agentKinds`); a kind named `registry-<id>` is
// joined to its line here by `kind` for its version, licence, repository and icon. Everything is still treated as third-party text.
import catalog from '../../../../manifests/agents/registry/catalog.json';
import { plainText, REGISTRY_PREFIX } from './agentKinds';

export { isRegistryKind, REGISTRY_PREFIX } from './agentKinds';

/** How a registry kind starts: through npx or uvx (which fetch the pinned package the first time), or as its own program. */
export type RegistryRuns = 'npx' | 'uvx' | 'binary';

export interface RegistryInfo {
  kind: string; id: string; name: string; version: string; runs: RegistryRuns;
  license?: string; licenseUrl?: string; repository?: string; website?: string;
  /** A URL of the vendored icon (a plain one-colour drawing, checked at conversion), or none. */
  icon?: string;
  /** Always false in this step: nobody has run it through Agora yet. */
  tested: boolean;
}

/** A link a person may follow from the list: `https:` only, no user name or password, at most 300 characters. Anything else is
 * shown as nothing rather than as a link. */
export function safeLink(raw: unknown): string | undefined {
  if (typeof raw !== 'string' || raw.length > 300) return undefined;
  try {
    const u = new URL(raw);
    return u.protocol === 'https:' && !u.username && !u.password && u.hostname.includes('.') ? u.href : undefined;
  } catch { return undefined; }
}

const icons = import.meta.glob('../../../../manifests/agents/registry/icons/*.svg', { eager: true, query: '?url', import: 'default' }) as Record<string, string>;
const iconOf = (id: string): string | undefined => icons[`../../../../manifests/agents/registry/icons/${id}.svg`];
/** The URL of the vendored registry icon `id`, when there is one (also the built-in kinds' marks, `kindIcons.ts`). */
export const registryIcon = iconOf;

const str = (v: unknown, max: number): string | undefined => (typeof v === 'string' && v.trim() ? plainText(v, max) : undefined);

/** One catalog line, cleaned; `undefined` for a line that is not one. Exported for the tests. */
export function cleanLine(v: unknown): RegistryInfo | undefined {
  if (typeof v !== 'object' || v === null) return undefined;
  const r = v as Record<string, unknown>;
  const kind = typeof r.kind === 'string' ? r.kind : '';
  const id = typeof r.id === 'string' ? r.id : '';
  const name = str(r.name, 48);
  const version = str(r.version, 32);
  if (!/^[a-z0-9_-]{1,64}$/.test(id) || kind !== `${REGISTRY_PREFIX}${id}` || !name || !version) return undefined;
  const runs: RegistryRuns = r.runs === 'npx' || r.runs === 'uvx' ? r.runs : 'binary';
  const license = str(r.license, 40);
  const licenseUrl = safeLink(r.license_url);
  const repository = safeLink(r.repository);
  const website = safeLink(r.website);
  const icon = r.icon === true ? iconOf(id) : undefined;
  return { kind, id, name, version, runs, tested: false, ...(license ? { license } : {}), ...(licenseUrl ? { licenseUrl } : {}), ...(repository ? { repository } : {}), ...(website ? { website } : {}), ...(icon ? { icon } : {}) };
}

const byKind: ReadonlyMap<string, RegistryInfo> = new Map(
  (Array.isArray(catalog.agents) ? catalog.agents : []).map(cleanLine).filter((l): l is RegistryInfo => !!l).map((l) => [l.kind, l]),
);

/** The registry line of a kind, when it is one this window knows. */
export const registryInfo = (kind: string): RegistryInfo | undefined => byKind.get(kind);

/** How many agents the built-in catalog lists, and the registry commit it was made from. */
export const registryCount = (): number => byKind.size;
export const registryCommit = (): string => (typeof catalog.registry_commit === 'string' ? catalog.registry_commit.slice(0, 12) : '');
