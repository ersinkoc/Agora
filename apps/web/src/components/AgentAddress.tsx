import { useState } from 'react';
import { t } from '../i18n';
import type { AgentTileModel } from '../model';

// The hub-qualified id is the routing address. Never manufacture one from a host/display name.
export function AgentAddress({ agent }: { agent: AgentTileModel }) {
  const [state, setState] = useState<'copied' | 'failed'>();
  if (!agent.machineId) return null;
  const address = agent.id;
  const copy = async () => {
    try { await navigator.clipboard.writeText(address); setState('copied'); }
    catch { setState('failed'); }
  };
  return <div className="flex flex-wrap items-center gap-x-2 gap-y-1 border-b border-line bg-surface-1 px-3 py-2 text-xs">
    <span className="text-muted">{t('agent-address')}</span>
    <code dir="ltr" className="min-w-0 select-all break-all">{address}</code>
    <button type="button" className="button px-2 py-1 text-xs" aria-label={`${t('agent-address-copy')} ${address}`} onClick={() => void copy()}>{t(state === 'copied' ? 'address-copied' : 'address-copy')}</button>
    <span role="status" className={state === 'failed' ? 'text-muted' : 'sr-only'}>{state ? t(state === 'copied' ? 'address-copied' : 'address-copy-failed') : ''}</span>
    {agent.parentName && <span className="ms-auto text-muted">{t('helper-started-by', { name: agent.parentName })}</span>}
  </div>;
}
