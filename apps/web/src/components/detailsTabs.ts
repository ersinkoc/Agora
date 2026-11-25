// The details panel's sections, in one list: the panel's title and the vertical menu at the window's end
// read the same list, and the layout's own panels (ADR-0068) draw the same sections.
import { t } from '../i18n';
import type { IconName } from './Icon';
import type { AgentTileModel } from '../model';
import type { UsageSummary } from '../net/usage';
import type { ApprovalModel, BoardModel, MessageModel } from '../net/view';
import type { PairingUi } from './Pairing';

export type DetailsTab = 'needs' | 'changes' | 'board' | 'messages' | 'cost';

export type DetailsSection = { id: DetailsTab; label: string; badge?: number; icon: IconName };

export function detailsSections(input: {
  attention: readonly AgentTileModel[];
  approvals: readonly ApprovalModel[];
  changed: readonly AgentTileModel[];
  boards: readonly BoardModel[];
  messages: readonly MessageModel[];
  usage?: UsageSummary;
  pairing?: PairingUi;
}): [DetailsSection, ...DetailsSection[]] {
  const { attention, approvals, changed, boards, messages, usage, pairing } = input;
  // An agent that waits and has an approval open is one thing that needs you, not two.
  const needs = attention.length + approvals.filter((x) => !attention.some((a) => a.id === x.agentId)).length + (pairing?.cards.length ?? 0);
  const rest: DetailsSection[] = [
    { id: 'changes', label: t('ide-tab-changes'), badge: changed.length || undefined, icon: 'pencil' },
    ...(boards.length > 0 ? [{ id: 'board' as const, label: t('board-title'), icon: 'files' as const }] : []),
    { id: 'messages', label: t('ide-tab-messages'), badge: messages.length || undefined, icon: 'message' },
    ...(usage && usage.totalMicroUsd > 0 ? [{ id: 'cost' as const, label: t('ide-tab-cost'), icon: 'chart' as const }] : []),
  ];
  // What needs you comes first: it is the section that opens, and the one that wins when something new waits.
  return [{ id: 'needs', label: t('attention-title'), badge: needs || undefined, icon: 'bell' }, ...rest];
}
