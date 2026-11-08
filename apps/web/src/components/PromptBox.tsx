import { useState } from 'react';
import { t } from '../i18n';
import type { AgentTileModel } from '../model';

// A chat agent has no terminal: you talk to it with requests. What it says back is not streamed to the window yet (that needs
// a protocol addition); its state, approvals, files and changes show everywhere else.
export function PromptBox({ agent, send }: { agent: AgentTileModel; send?: (a: AgentTileModel, text: string) => Promise<boolean> }) {
  const [text, setText] = useState('');
  const [sent, setSent] = useState<'ok' | 'failed' | undefined>();
  const [busy, setBusy] = useState(false);
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
    <form className="flex h-full flex-col gap-3 rounded-m bg-surface-0 p-4" onSubmit={(e) => { e.preventDefault(); void go(); }}>
      <p className="muted m-0">{t('prompt-intro')}</p>
      <textarea value={text} onChange={(e) => setText(e.target.value)} aria-label={t('prompt-label')} placeholder={t('prompt-label')}
        className="min-h-24 w-full flex-1 resize-none rounded-s border border-line bg-surface-1 p-3 text-ink"
        onKeyDown={(e) => { if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); void go(); } }} />
      <div className="flex items-center gap-3">
        <button type="submit" className="button button--primary" disabled={busy || !text.trim() || !send}>{t('prompt-send')}</button>
        {sent && <span role="status" className={`text-xs ${sent === 'ok' ? 'text-finished' : 'text-needs-you'}`}>{t(sent === 'ok' ? 'prompt-sent' : 'prompt-failed')}</span>}
      </div>
    </form>
  );
}
