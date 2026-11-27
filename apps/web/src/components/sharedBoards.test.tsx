import { renderToStaticMarkup } from 'react-dom/server';
import { expect, it } from 'vitest';
import { agora } from '../gen/agora.js';
import { setLanguage } from '../i18n';
import { applyEvent, boards, fromSnapshot } from '../net/view';
import { acceptedTaskNotices, SharedBoards, ShowAcceptedTasks } from './SharedBoards';

const review = (state: agora.v1.TaskReviewState, decidedAtMs = 1717171717000) => ({ reportRevision: 2, state, decidedAtMs });
const { TASK_REVIEW_STATE_ACCEPTED: accepted, TASK_REVIEW_STATE_RETURNED: returned, TASK_REVIEW_STATE_PENDING: pending } = agora.v1.TaskReviewState;

it('derives accepted rows from snapshots and their positioned overflow without duplicate or unaccepted tasks', () => {
  const v = fromSnapshot({ pos: 7, reviewEnabledProjectIds: ['p'], tasks: [
    { projectId: 'p', taskId: 'old', title: 'Legacy done task', done: true },
    { projectId: 'p', taskId: 'returned', title: 'Returned task', review: review(returned) },
    { projectId: 'p', taskId: 'pending', title: 'Pending task', review: review(pending) },
    { projectId: 'p', taskId: 'not-done', title: 'Still open', review: review(accepted) },
    { projectId: 'p', taskId: 'finished', title: 'Accepted work', done: true, review: review(accepted, 1717171716000) },
    { projectId: 'p', taskId: 'replayed', title: 'Accepted after replay', done: true },
  ] });
  const list = () => acceptedTaskNotices(boards(v)).map(row => [row.task.taskId, row.revision]);
  const arrival = { body: 'taskReviewChanged' as const, taskReviewChanged: { projectId: 'p', taskId: 'replayed', review: review(accepted) } };
  applyEvent(v, 7, { ...arrival, producerId: 'machine' });
  expect(list()).toEqual([['finished', 2]]);
  applyEvent(v, 7, { ...arrival, producerId: 'hub' });
  applyEvent(v, 8, { ...arrival, producerId: 'hub' });
  expect(list()).toEqual([['replayed', 2], ['finished', 2]]);

  expect(acceptedTaskNotices(boards(fromSnapshot({
    tasks: [{ projectId: 'p', taskId: 'old', title: 'Only legacy Done', done: true }],
  })))).toEqual([]);
});

it('uses newest decision time first, breaks ties by project and task IDs, and bounds the list at 50', () => {
  const tasks = Array.from({ length: 52 }, (_, i) => ({
    projectId: 'z', taskId: String(i).padStart(2, '0'), done: true, review: review(accepted),
  }));
  tasks.reverse();
  tasks.push({ projectId: 'a', taskId: '99', done: true, review: review(accepted) });
  tasks.push({ projectId: 'z', taskId: '98', done: true, review: review(accepted, 1717171718000) });
  const notices = acceptedTaskNotices(boards(fromSnapshot({ reviewEnabledProjectIds: ['a', 'z'], tasks })));
  expect(notices).toHaveLength(50);
  expect(notices.slice(0, 4).map(row => row.task.id)).toEqual(['z/98', 'a/99', 'z/00', 'z/01']);
  expect(notices[49]?.task.id).toBe('z/47');
});

it('renders the notice, a machine-readable acceptance time and plain-text task title only for the owner', () => {
  setLanguage('en');
  const acceptedAt = 1717171717000;
  const list = boards(fromSnapshot({
    reviewEnabledProjectIds: ['p'],
    tasks: [{ projectId: 'p', taskId: '1', title: '<b>literal title</b>', done: true, review: review(accepted, acceptedAt) }],
  }));
  const html = (owner: boolean) => renderToStaticMarkup(
    <ShowAcceptedTasks.Provider value={owner}><SharedBoards boards={list} /></ShowAcceptedTasks.Provider>,
  );
  expect(html(false)).not.toContain('Recently accepted');
  expect(html(true)).toContain('Recently accepted');
  expect(html(true)).toContain(new Date(acceptedAt).toISOString());
  expect(html(true)).toContain('&lt;b&gt;literal title&lt;/b&gt;');
  expect(html(true)).not.toContain('<b>literal title</b>');
});
