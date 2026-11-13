import { useEffect, useRef, useState } from 'react';
import { t } from '../i18n';
import type { AgentTileModel } from '../model';
import { mergeLines, noticeText, type ChatLine, type ChatSource } from '../net/chat';
import { SessionBar } from './SessionBar';
import type { OptionKind } from './sessionChoice';

function Line({ line }: { line: ChatLine }) {
  if (line.role === 'user') {
    return <div className="ms-auto max-w-[85%] rounded-m bg-accent/15 px-3 py-2 break-words whitespace-pre-wrap">{line.text}</div>;
  }
  if (line.kind === 'notice') {
    // Something went wrong or changed in the conversation (ADR-0058): said by the window, from the code the agent's adapter sent.
    return <div role="note" className="max-w-[90%] rounded-s border border-needs-you/50 bg-surface-1 px-3 py-1.5 text-xs break-words whitespace-pre-wrap text-muted">{noticeText(line, t)}</div>;
  }
  if (line.kind === 'tool') {
    return <div className="max-w-[90%] rounded-s border border-line bg-surface-0 px-3 py-1.5 font-mono text-xs break-all text-muted"><span className="font-sans font-medium text-ink">{t('chat-tool')}</span> {line.text}</div>;
  }
  if (line.kind === 'plan') {
    return (
      <div className="max-w-[90%] rounded-s border border-line px-3 py-2 text-[13px]">
        <div className="muted mb-1">{t('chat-plan')}</div>
        <ol className="m-0 list-decimal ps-5">{line.text.split('\n').map((p, i) => <li key={i}>{p}</li>)}</ol>
      </div>
    );
  }
  if (line.kind === 'diff') {
    return (
      <div className="max-w-full overflow-hidden rounded-s border border-line">
        <div className="border-b border-line bg-surface-2 px-3 py-1 font-mono text-xs">{line.path}</div>
        <pre className="m-0 max-h-60 overflow-auto p-2 font-mono text-xs">{line.text}</pre>
      </div>
    );
  }
  return <div className="max-w-[90%] break-words whitespace-pre-wrap">{line.text}</div>;
}

// A chat agent has no terminal: you talk to it with requests and read its answers, as a conversation the node keeps (ADR-0038).
export function PromptBox({ agent, send, source, choose }: { agent: AgentTileModel; send?: (a: AgentTileModel, text: string) => Promise<boolean>; source?: ChatSource; choose?: (agentKey: string, kind: OptionKind, id: string) => Promise<boolean> }) {
  const [text, setText] = useState('');
  const [sent, setSent] = useState<'ok' | 'failed' | undefined>();
  const [busy, setBusy] = useState(false);
  const [lines, setLines] = useState<ChatLine[]>([]);
  const end = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!source) return;
    setLines([]);
    const sub = source.connect((incoming, replace) => setLines((held) => (replace ? incoming : mergeLines(held, incoming))));
    return () => sub.close();
  }, [source]);
  useEffect(() => { end.current?.scrollIntoView?.({ block: 'end' }); }, [lines.length]);

  const go = async () => {
    const body = text.trim();
    if (!body || !send) return;
    setBusy(true);
    const ok = await send(agent, body);
    setBusy(false);
    setSent(ok ? 'ok' : 'failed');
    if (ok) setText('');
  };
  return (
    <div className="flex h-full min-h-0 flex-col gap-3 rounded-m bg-surface-0 p-3">
      {/* Keyed by the agent: another agent on screen gets a fresh bar, so nothing left open carries over; usage updates keep it. */}
      {agent.session && <SessionBar key={agent.id} agentKey={agent.id} session={agent.session} disabled={agent.closed} choose={choose} />}
      <div className="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto" aria-live="polite" aria-label={t('chat-log')}>
        {lines.length === 0 && <p className="muted m-0">{t(source ? 'chat-empty' : 'prompt-intro')}</p>}
        {lines.map((l) => <Line key={l.id} line={l} />)}
        <div ref={end} />
      </div>
      <form className="flex flex-none flex-col gap-2" onSubmit={(e) => { e.preventDefault(); void go(); }}>
        <textarea value={text} onChange={(e) => setText(e.target.value)} aria-label={t('prompt-label')} placeholder={t('prompt-label')} rows={3}
          className="w-full resize-none rounded-s border border-line bg-surface-1 p-3 text-ink"
          onKeyDown={(e) => { if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); void go(); } }} />
        <div className="flex items-center gap-3">
          <button type="submit" className="button button--primary" disabled={busy || !text.trim() || !send}>{t('prompt-send')}</button>
          {sent && <span role="status" className={`text-xs ${sent === 'ok' ? 'text-finished' : 'text-needs-you'}`}>{t(sent === 'ok' ? 'prompt-sent' : 'prompt-failed')}</span>}
        </div>
      </form>
    </div>
  );
}
