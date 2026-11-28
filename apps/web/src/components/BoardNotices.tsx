import { createContext, useContext, useEffect, useId, useState } from 'react';
import { t } from '../i18n';
import { visible } from '../net/hidden';
import type { NoticeSettings } from '../net/notices';

/**
 * ADR-0130 Amendment A: what the owner's window offers for a project's channel notices. `channels` are the project's existing channels
 * (the `#name`s its agents wrote to in recent Team chat); `save` is left out while the link is not live. Given only to the owner's
 * window (the hub answers these commands to the owner alone in this step).
 */
export interface BoardNoticesApi {
  channels: (projectId: string) => readonly string[];
  read: (projectId: string) => Promise<NoticeSettings>;
  save?: (projectId: string, read: NoticeSettings, channel: string) => Promise<NoticeSettings>;
}
export const BoardNoticesContext = createContext<BoardNoticesApi | undefined>(undefined);

const REASONS: Record<string, string> = { review_off: 'board-notices-paused-review', not_shared: 'board-notices-paused-shared' };

/** Where a Review-enabled project's accepted tasks are posted: nowhere, or one of its channels. Drawn on the project's board card. */
export function BoardNotices({ projectId }: { projectId: string }) {
  const api = useContext(BoardNoticesContext);
  const id = useId();
  const [settings, setSettings] = useState<NoticeSettings>();
  const [state, setState] = useState<'reading' | 'ready' | 'unread' | 'saving' | 'failed'>('reading');
  useEffect(() => {
    if (!api) return;
    let stop = false;
    setState('reading');
    setSettings(undefined);
    api.read(projectId).then((s) => { if (!stop) { setSettings(s); setState('ready'); } }).catch(() => { if (!stop) setState('unread'); });
    return () => { stop = true; };
  }, [api?.read, projectId]); // eslint-disable-line react-hooks/exhaustive-deps
  if (!api || state === 'reading' || state === 'unread' || !settings) return null;
  // The saved channel is always offered, also once it has rolled out of recent Team chat: opening this never changes it.
  const seen = api.channels(projectId);
  const options = settings.channel && !seen.includes(settings.channel) ? [settings.channel, ...seen] : [...seen];
  const choose = (channel: string) => {
    const save = api.save;
    if (!save || channel === settings.channel) return;
    setState('saving');
    save(projectId, settings, channel).then((s) => { setSettings(s); setState('ready'); }).catch(() => setState('failed'));
  };
  const paused = settings.channel && !settings.active;
  return (
    <div className="grid min-w-0 gap-1 border-t border-line pt-2 text-xs" data-board-notices>
      <label htmlFor={id} className="text-muted">{t('board-notices-label')}</label>
      <select id={id} className="w-full rounded-xs py-1 text-xs" value={settings.channel} disabled={!api.save || state === 'saving'} onChange={(e) => choose(e.target.value)}>
        <option value="">{t('board-notices-off')}</option>
        {options.map((c) => <option key={c} value={c}>{visible(c)}</option>)}
      </select>
      {options.length === 0 && <p className="m-0 text-muted">{t('board-notices-no-channels')}</p>}
      {paused && <p className="m-0 text-muted">{t(REASONS[settings.inactiveReason] ?? 'board-notices-paused')}</p>}
      {state === 'failed' && <p role="alert" className="m-0 text-danger">{t('board-notices-failed')}</p>}
    </div>
  );
}
