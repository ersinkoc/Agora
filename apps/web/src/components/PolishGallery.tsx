import { t } from '../i18n';
import type { AgentTileModel, SessionModel } from '../model';
import type { ChatLine, ChatSource } from '../net/chat';
import type { QuotaObs } from '../net/quota';
import type { ApprovalModel, BoardModel } from '../net/view';
import { ApprovalBanner } from './ApprovalBanner';
import { BoardColumns } from './BoardView';
import { GiveContext, type GiveApi } from './GiveTask';
import { PromptBox } from './PromptBox';
import { QuotaProvider } from './Quota';
import { UsagePanel } from './UsagePanel';

// Redesign PR 12 in the gallery: a chat agent idle, needing you and acting freely; the board view's columns; the Usage panel with a plan
// limit at 82 %. Fixed times and a fixed clock, so a capture is the same every time (the visual comparison and docs/ux/screens/polish-redesign-*).
const AT = new Date(2026, 10, 22, 21, 33).getTime();
const choice = (id: string, name: string, skips = false) => ({ id, name, description: '', skipsPermissions: skips });
const session = (mode: string): SessionModel => ({
  modes: [choice('default', 'Default'), choice('plan', 'Plan'), choice('bypassPermissions', 'Bypass Permissions', true)],
  models: [choice('large', 'Opus 5.5'), choice('small', 'Haiku 5')], currentMode: mode, currentModel: 'large', contextTokens: 0, contextWindow: 0,
});
const chatAgent = (over: Partial<AgentTileModel>): AgentTileModel => ({
  id: 'polish/chat', name: 'claude · amber', kind: 'claude', accent: 'var(--agent-amber)', machine: 'laptop', machineId: 'laptop', workspace: 'login-fix',
  project: 'login-fix', projectId: 'polish', state: 'idle', preview: '', terminal: false, tokens: 215, session: session('default'), ...over,
});
const line = (id: number, role: ChatLine['role'], text: string, over: Partial<ChatLine> = {}): ChatLine => ({ id, atMs: AT + id * 1000, role, kind: 'text', text, path: '', ...over });
const source = (lines: ChatLine[]): ChatSource => ({ connect(on) { on(lines, true); return { close() {} }; } });

const idle = source([
  line(1, 'user', 'Refactor the session store so tokens expire after fifteen minutes.'),
  line(2, 'agent', 'Done: tokens now expire after fifteen minutes, and the expired and wrong-signature cases have tests.'),
]);
const asking = source([line(1, 'user', 'Run the login tests.'), line(2, 'agent', 'run_command', { kind: 'permission', approvalId: 'polish/1', outcome: 'pending' })]);
const free = source([line(1, 'user', 'Switch to the mode that does not ask, then fix the lint warnings.'), line(2, 'agent', 'Switched. Fixing the warnings now.')]);
const ask: ApprovalModel = { id: 'polish/1', node: 'laptop', localId: '1', agentId: 'polish/needs', agentName: 'claude · amber', action: 'run_command', detail: 'cargo test login', expiresAtMs: 0 };

const board: BoardModel = {
  projectId: 'polish', project: 'login-fix', folders: [], leases: [],
  tasks: [
    { id: 't1', title: 'Write the refresh-token path in src/refresh.rs', status: 'open', owner: '' },
    { id: 't2', title: 'Investigate why tokens from the build server are refused on the laptop', status: 'open', owner: '' },
    { id: 't3', title: 'Pass the clock into Token::is_expired', status: 'claimed', owner: 'codex · teal', ownerState: 'working', held: ['src/auth.rs'] },
    { id: 't4', title: 'Update the README for short-lived tokens', status: 'done', owner: 'claude · violet' },
  ],
};
const give: GiveApi = { agents: [chatAgent({ id: 'laptop/a1' })], online: new Set(['laptop']), machines: [{ id: 'laptop', name: 'laptop' }], send: async () => ({ ok: true, queued: false }) };

const usageFleet: AgentTileModel[] = [
  chatAgent({ id: 'laptop/a1', tokens: 48_200, startedMs: AT - 55 * 60_000, session: { ...session('default'), contextTokens: 61_000, contextWindow: 200_000 } }),
  chatAgent({ id: 'laptop/a2', name: 'codex · teal', kind: 'codex', accent: 'var(--agent-teal)', tokens: 21_000, startedMs: AT - 34 * 60_000, session: { ...session('default'), contextTokens: 172_000, contextWindow: 200_000 } }),
  chatAgent({ id: 'build/a3', name: 'claude · violet', accent: 'var(--agent-violet)', machine: 'build', machineId: 'build', project: 'api', projectId: 'api', tokens: 9_100, startedMs: AT - 20 * 60_000, closed: true, endedMs: AT - 5 * 60_000 }),
];
// The plan limits read the same fixed clock (`fixedNow`), so "Resets in…" and "Observed…" never move.
const quota: QuotaObs[] = [
  { machineId: 'laptop', tool: 'codex', window: 'five_hour', usedPercent: 82, resetsAtMs: AT + 95 * 60_000, observedAtMs: AT - 4 * 60_000 },
  { machineId: 'laptop', tool: 'codex', window: 'seven_day', usedPercent: 41, resetsAtMs: AT + 3 * 86_400_000, observedAtMs: AT - 4 * 60_000 },
];

export function PolishGallery() {
  return (
    <div data-polish className="grid grid-cols-1 gap-4">
      <div data-polish-chat="idle" className="h-[22rem] max-w-2xl"><PromptBox agent={chatAgent({})} source={idle} send={async () => true} choose={async () => true} /></div>
      {/* Taller on a phone: the pinned request card is taller there, and the log above it still shows an entry whole. */}
      <div data-polish-chat="needs-you" className="h-[26rem] max-w-2xl max-md:h-[32rem]">
        <PromptBox agent={chatAgent({ id: 'polish/needs', state: 'needs-you' })} source={asking} send={async () => true} choose={async () => true} approvals={<ApprovalBanner approval={ask} onResolve={() => {}} pinned />} />
      </div>
      {/* #1082: a pane too short for the card and the conversation: the request is one row (Allow and Deny beside the whole ask where it
          fits; on a phone the title and Review, which opens the card). A little taller on a phone, still short, so its log shows an entry. */}
      <div data-polish-chat="short" className="h-[20rem] max-w-2xl max-md:h-[24rem]">
        <PromptBox agent={chatAgent({ id: 'polish/needs', state: 'needs-you' })} source={asking} send={async () => true} choose={async () => true} approvals={<ApprovalBanner approval={ask} onResolve={() => {}} pinned />} />
      </div>
      <div data-polish-chat="acts-freely" className="h-[24rem] max-w-2xl"><PromptBox agent={chatAgent({ id: 'polish/free', session: session('bypassPermissions') })} source={free} send={async () => true} choose={async () => true} /></div>
      <div data-polish-board className="flex h-[26rem] max-w-4xl flex-col overflow-hidden rounded-l border border-line" aria-label={t('board-view-title', { name: board.project })}>
        <GiveContext.Provider value={give}><BoardColumns board={board} /></GiveContext.Provider>
      </div>
      <div data-polish-usage className="max-w-sm rounded-l border border-line bg-surface-1 p-4">
        <QuotaProvider obs={quota} machines={new Map([['laptop', 'laptop'], ['build', 'build']])} fixedNow={AT}><UsagePanel fleet={usageFleet} now={AT} /></QuotaProvider>
      </div>
    </div>
  );
}
