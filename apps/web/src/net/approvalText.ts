import { t } from '../i18n';
import type { ApprovalModel } from './view';

// Who asks, in words a person knows: the agent's display name, else a plain word. Never an id and never an empty string.
export const approvalName = (a: Pick<ApprovalModel, 'agentName'>): string => a.agentName.trim() || t('approval-someone');
