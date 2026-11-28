// ADR-0130 Amendment A: accepted-task notices in a project's channel, kept by the hub as rows and read in bounded pages. The event stream
// carries only a content-free hint per project (`ProjectNoticesChanged`, folded into `View.noticeHints`); the window reads the project's
// rows again when it connects and after each hint. In this step the three commands are the owner's alone (unscoped), so a member's or
// viewer's window never sends them, and a refusal (a scoped owner, a hub that predates them) stops asking until the next connection:
// every refused owner-only command is a row in the owner's Activity.
import { useEffect, useRef, useState } from 'react';
import type { agora } from '../gen/agora.js';
import { CommandError, type CommandBody } from './live';
import type { BoardModel, View } from './view';
import { num } from './view';

type Command = (node: string, body: CommandBody) => Promise<agora.v1.CommandOk.$Properties>;

/** One accepted task as the channel shows it. No dispatcher or reviewer identity (decision 1). */
export interface ProjectNoticeModel { id: string; projectId: string; taskId: string; revision: number; acceptedMs: number; channel: string; createdMs: number }

/** The settings as the hub says them: `inactiveReason` is `review_off`, `not_shared`, or a reason a newer hub may add. */
export interface NoticeSettings { dispatcher: boolean; channel: string; active: boolean; inactiveReason: string }

/** Rows per page (the hub's most) and pages per read: the hub keeps 50 per project channel, so a few pages cover a changed channel too. */
export const NOTICE_PAGE = 50;
export const NOTICE_PAGES = 4;

/** Whether an error is the hub's answer (refused, unknown command) rather than the link going away; only an answer stops asking. */
export const isRefusal = (e: unknown): boolean => e instanceof CommandError && e.code !== 'node.offline';

function noticeOf(n: agora.v1.ProjectNotice.$Properties): ProjectNoticeModel {
  return { id: n.noticeId ?? '', projectId: n.projectId ?? '', taskId: n.taskId ?? '', revision: num(n.reportRevision), acceptedMs: num(n.acceptedMs), channel: n.channel ?? '', createdMs: num(n.createdMs) };
}

/** A project's notices from the start, following `more` for at most `NOTICE_PAGES` pages (oldest first, as the hub pages them). */
export async function readNotices(command: Command, projectId: string): Promise<ProjectNoticeModel[]> {
  const rows: ProjectNoticeModel[] = [];
  let afterCreatedMs = 0;
  let afterNoticeId = '';
  for (let page = 0; page < NOTICE_PAGES; page++) {
    const r = await command('', { projectNoticesPage: { projectId, afterCreatedMs, afterNoticeId, limit: NOTICE_PAGE } });
    const got = (r.projectNoticesPage?.notices ?? []).map(noticeOf).filter((n) => n.projectId === projectId && n.id);
    rows.push(...got);
    const last = got[got.length - 1];
    if (!r.projectNoticesPage?.more || !last) break;
    afterCreatedMs = last.createdMs;
    afterNoticeId = last.id;
  }
  return rows;
}

export async function readNoticeSettings(command: Command, projectId: string): Promise<NoticeSettings> {
  const r = await command('', { projectNoticesSettingsGet: { projectId } });
  const s = r.projectNoticesSettings;
  if (!s) throw new CommandError('protocol.invalid_message');
  return { dispatcher: !!s.dispatcher, channel: s.channel ?? '', active: !!s.active, inactiveReason: s.inactiveReason ?? '' };
}

/** Saves the channel (empty turns channel notices off), keeping the `dispatcher` setting as it was read. The hub answers the new settings. */
export async function saveNoticeChannel(command: Command, projectId: string, read: NoticeSettings, channel: string): Promise<NoticeSettings> {
  const r = await command('', { projectNoticesSettingsSet: { projectId, dispatcher: read.dispatcher, channel } });
  const s = r.projectNoticesSettings;
  if (!s) return readNoticeSettings(command, projectId);
  return { dispatcher: !!s.dispatcher, channel: s.channel ?? '', active: !!s.active, inactiveReason: s.inactiveReason ?? '' };
}

/** What the reads depend on: the Review-enabled projects and each one's last hint, as one stable string. */
export function noticeReads(v: View | undefined): [string, number][] {
  if (!v) return [];
  return [...v.reviewProjects].sort().map((p) => [p, v.noticeHints?.get(p) ?? 0]);
}

/**
 * The owner's notices of every Review-enabled project. `connected` is the live link: each time it comes back every project is read
 * again (the snapshot carries no notices). A hint re-reads that project from the start and replaces its rows, so the hub's expiry and
 * eviction show here too. Projects are read one at a time; a project's older answer never replaces a newer one.
 */
export function useProjectNotices(command: Command | undefined, view: View | undefined, owner: boolean, connected: boolean): ProjectNoticeModel[] {
  const [rows, setRows] = useState<ReadonlyMap<string, ProjectNoticeModel[]>>(new Map());
  const read = useRef(new Map<string, number>()); // project -> the hint position its last read started at
  const generation = useRef(new Map<string, number>());
  const silenced = useRef(false);
  const queue = useRef<Promise<void>>(Promise.resolve());
  const asks = !!command && owner && connected;
  const wanted = noticeReads(view);
  const key = wanted.map(([p, h]) => `${p}\u0000${h}`).join('\n');
  useEffect(() => {
    // A new connection (or a lost one, or another role) starts again.
    read.current.clear();
    silenced.current = false;
    if (!asks) setRows(new Map());
  }, [asks, command]);
  useEffect(() => {
    if (!asks || !command) return;
    for (const [project, hint] of wanted) {
      if (read.current.get(project) === hint) continue;
      read.current.set(project, hint);
      const gen = (generation.current.get(project) ?? 0) + 1;
      generation.current.set(project, gen);
      queue.current = queue.current.then(async () => {
        if (silenced.current || generation.current.get(project) !== gen) return;
        try {
          const got = await readNotices(command, project);
          if (generation.current.get(project) !== gen) return;
          setRows((m) => new Map(m).set(project, got));
        } catch (e) {
          if (isRefusal(e)) silenced.current = true;
          else if (read.current.get(project) === hint) read.current.delete(project); // the link went away: read again when it is back
        }
      });
    }
  }, [asks, command, key]); // eslint-disable-line react-hooks/exhaustive-deps
  return asks ? [...rows.values()].flat() : [];
}

/** A notice as a Team chat row: the board's task title (untrusted, shown as plain text by the row) and the project's name. */
export interface NoticeRowModel { id: string; channel: string; title?: string; project: string; atMs: number }

/** The newest `limit` notices, with titles from the boards; a task the window has not seen yet has none (the row says so). */
export function noticeRows(notices: readonly ProjectNoticeModel[], boards: readonly BoardModel[], limit = 5): NoticeRowModel[] {
  const byProject = new Map(boards.map((b) => [b.projectId, b]));
  return [...notices]
    .sort((a, b) => b.acceptedMs - a.acceptedMs || b.createdMs - a.createdMs || (a.id < b.id ? 1 : a.id > b.id ? -1 : 0))
    .slice(0, limit)
    .map((n) => {
      const board = byProject.get(n.projectId);
      const title = board?.tasks.find((t) => t.taskId === n.taskId)?.title;
      return { id: n.id, channel: n.channel, project: board?.project ?? n.projectId, atMs: n.acceptedMs, ...(title ? { title } : {}) };
    });
}
