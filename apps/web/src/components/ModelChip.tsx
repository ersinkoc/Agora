import { t } from '../i18n';
import type { AgentTileModel } from '../model';
import { visible } from '../net/view';
import { plain } from './boardColumns';

/** A provider's name in the UI's words (`provider-<id>` in the locale files), else its id as the manifest wrote it. */
export function providerName(id: string): string {
  if (!id) return '';
  const key = `provider-${id}`;
  const name = t(key);
  return name === key ? id : name;
}

/** The short line ("opus · high") and the whole of it in words (provider, model, effort), or `undefined` when nothing is known. */
export function modelWords(agent: Pick<AgentTileModel, 'model' | 'effort' | 'provider'>): { short: string; full: string } | undefined {
  const model = visible(plain(agent.model ?? '', 48));
  const effort = visible(plain(agent.effort ?? '', 24));
  if (!model && !effort) return undefined;
  const provider = providerName(agent.provider ?? '');
  const full = [provider && t('agent-provider-title', { value: provider }), model && t('agent-model-title', { value: model }), effort && t('agent-effort-title', { value: effort })]
    .filter(Boolean).join(' · ');
  const short = model ? `${model}${effort ? ` · ${effort}` : ''}` : t('agent-effort-chip', { value: effort });
  return { short, full };
}

// ADR-0123: the model an agent runs on and its reasoning effort ("opus · high"), beside its other chips. Both are the agent's own words
// (or the launch settings it was started with), so they are shown as one short plain line; the whole of both, with the provider's name,
// is the tooltip. Nothing when neither is known. `nested`: inside a button (a fleet tile), so it takes no focus of its own and its words
// are the browser's tooltip.
export function ModelChip({ agent, nested }: { agent: Pick<AgentTileModel, 'model' | 'effort' | 'provider'>; nested?: boolean }) {
  const words = modelWords(agent);
  if (!words) return null;
  const props = nested ? { title: words.full } : { tabIndex: 0, 'data-tip': words.full };
  return (
    <span role="img" aria-label={words.full} data-model-chip className="tip ide-chip w-fit max-w-[24ch] flex-none self-center justify-self-start" {...props}>
      <span aria-hidden className="truncate">{words.short}</span>
    </span>
  );
}
