// Test-only: a window's panel props with every built-in panel drawn by `draw` (nothing by default), for tests of what the window
// itself draws around its panes (tabs, pane bars, Show…, the empty pane's chooser).
import type { ReactNode } from 'react';
import type { AnswerGuard } from '../components/useAnswerGuard';
import type { AgentTileModel } from '../model';
import { BUILTIN_PANELS, panelRegistry, type PanelContext, type PanelDef, type PanelState } from './panels';
import type { WorkspaceProps } from './Workspace';

export function barePanels(fleet: readonly AgentTileModel[], draw: (ctx: PanelContext) => ReactNode = () => null) {
  const none = () => {};
  const panels = panelRegistry<PanelDef>(BUILTIN_PANELS.map((p): PanelDef => (p.kind === 'chooser' ? { ...p, kind: 'chooser' } : { ...p, kind: p.kind, render: draw })));
  const state: PanelState = { fleet, machines: [], approvals: [], attention: [], changed: [], messages: [], boards: [], projects: [], now: 0, link: 'live' };
  const props: Pick<WorkspaceProps, 'panels' | 'state' | 'sources' | 'actions' | 'terminalFor'> = {
    panels, state, sources: {},
    actions: { openAgent: none, openFromHome: none, openTool: none, review: none, history: none, browse: none, showNeeds: none, showChanges: none, dismiss: none, answers: {} as AnswerGuard, chat: {} },
    terminalFor: () => ({ connect: () => ({ sendInput() {}, resize() {}, close() {} }) }),
  };
  return props;
}
