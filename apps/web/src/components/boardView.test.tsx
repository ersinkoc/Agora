import { beforeEach, describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { setLanguage } from '../i18n';
import type { BoardModel, BoardTaskModel } from '../net/view';
import { COLUMN_PAGE, TITLE_MAX, columnsOf, plain } from './boardColumns';
import { BoardColumns } from './BoardView';
import { AcknowledgeHandoffContext } from './BoardHandoffs';

const task = (id: string, status: BoardTaskModel['status'], extra: Partial<BoardTaskModel> = {}): BoardTaskModel => ({ id, title: `T${id}`, status, owner: status === 'open' ? '' : 'Ada', ...extra });
const board = (tasks: BoardTaskModel[]): BoardModel => ({ projectId: 'p', project: 'proj', folders: [], tasks, leases: [] });

describe('board columns', () => {
  it('maps open, claimed and done to Backlog, Doing and Done in that order', () => {
    const cols = columnsOf([task('1', 'done'), task('2', 'open'), task('3', 'claimed'), task('4', 'open')]);
    expect(cols.map((c) => [c.id, c.cards.map((x) => x.id)])).toEqual([['backlog', ['2', '4']], ['doing', ['3']], ['done', ['1']]]);
  });
  it('caps a spammed column and counts the rest', () => {
    const many = Array.from({ length: 5000 }, (_, i) => task(String(i), 'open'));
    const backlog = columnsOf(many)[0]!;
    expect(backlog.total).toBe(5000);
    expect(backlog.cards).toHaveLength(COLUMN_PAGE);
    expect(columnsOf(many, { backlog: 400 })[0]!.cards).toHaveLength(400);
    const html = renderToStaticMarkup(<BoardColumns board={board(many)} />);
    expect((html.match(/data-task-id/g) ?? []).length).toBeLessThanOrEqual(COLUMN_PAGE);
  });
});

describe('untrusted task text', () => {
  it('makes hidden Unicode in one-line board text visible', () => {
    expect(plain('Ta\u3164sk\u034f', TITLE_MAX)).toBe('Ta\\u{3164}sk\\u{34f}');
  });
  it('verifies one-line board text across isolate, private-use and control characters', () => {
    expect(plain('模型\u2066', TITLE_MAX)).toBe('模型\\u{2066}');
    expect(plain('İş\ue000\u0001', TITLE_MAX)).toBe('İş\\u{e000}\\u{1}');
  });
  it('cuts to a short plain line and names control and bidirectional characters', () => {
    expect(plain('a‮b\u0007c\n\nd', 50)).toBe('a\\u{202e}b\\u{7}c d');
    const cut = plain('x'.repeat(10_000), TITLE_MAX);
    expect(Array.from(cut)).toHaveLength(TITLE_MAX);
    expect(cut.endsWith('…')).toBe(true);
  });
  for (const lang of ['en', 'tr'] as const) {
    it(`renders markup and links as text (${lang})`, () => {
      setLanguage(lang);
      const evil = task('1', 'open', { title: '<img src=x onerror=alert(1)> [click](https://evil.example) <b>hi</b>' });
      const html = renderToStaticMarkup(<BoardColumns board={board([evil, task('2', 'claimed', { owner: '<script>x</script>', held: ['a', 'b', 'c', 'd', 'e'], ownerSkips: true, ownerState: 'working' })])} />);
      expect(html).not.toContain('<img');
      expect(html).not.toContain('<script');
      expect(html).not.toContain('<b>');
      expect(html).not.toContain('<a ');
      expect(html).toContain('&lt;img');
      expect(html).not.toMatch(/>board-[a-z-]+</); // no message id leaks through
    });
  }
  it('says so when the board has no tasks, in the UI vocabulary', () => {
    setLanguage('en');
    const html = renderToStaticMarkup(<BoardColumns board={board([])} />);
    expect(html).toContain('No tasks yet');
    expect(html).not.toMatch(/claim|lease|mesh/i);
    // The kit's empty state (a tinted icon chip, a title and one line), and no empty columns under it.
    expect(html).toMatch(/class="kit-empty kit-tone-accent" role="status"><span aria-hidden="true" class="kit-chip">/);
    expect(html).toContain('Tasks that agents add for this project appear here.');
    expect(html).not.toContain('data-column');
  });
});

describe('the board look (redesign PR 12)', () => {
  beforeEach(() => setLanguage('en'));
  it('heads each column with its mark, hue and count, and draws the tasks as kit cards', () => {
    const html = renderToStaticMarkup(<BoardColumns board={board([task('1', 'open'), task('2', 'claimed', { ownerState: 'working' }), task('3', 'done')])} />);
    for (const [col, tone] of [['backlog', 'plain'], ['doing', 'working'], ['done', 'finished']]) {
      expect(html).toMatch(new RegExp(`data-column="${col}"[^>]*><h3 id="board-col-${col}" class="[^"]*kit-tone-${tone}`));
    }
    expect(html).toMatch(/<span class="kit-tag">1<\/span><\/h3>/);
    expect(html.match(/class="kit-card kit-card--dense/g)).toHaveLength(3);
    expect(html).toContain('kit-pill kit-tone-working'); // the owner's state as the kit pill
    expect(html).not.toMatch(/needs-you|tint-needs/); // nothing on the board is amber unless an agent needs you
  });
  it('says a column is empty in a quiet dashed frame', () => {
    const html = renderToStaticMarkup(<BoardColumns board={board([task('1', 'open')])} />);
    expect(html).toMatch(/border-dashed[^>]*>Nothing in progress\.</);
  });
});
beforeEach(() => setLanguage('en'));

it('shows unresolved handoffs and only offers acknowledgment when an action is available', () => {
  setLanguage('en');
  const data = board([task('1', 'claimed', { handoffs: [
    { id: 'one', receiver: 'West agent', state: 'unresolved', acknowledged: false, errorCode: '' },
    { id: 'two', receiver: 'Old agent', state: 'failed', acknowledged: false, errorCode: 'mesh.recipient_unknown' },
  ] })]);
  const columns = <BoardColumns board={data} />;
  expect(renderToStaticMarkup(columns)).not.toContain('Acknowledge unconfirmed handoff');
  const html = renderToStaticMarkup(<AcknowledgeHandoffContext.Provider value={async () => {}}>{columns}</AcknowledgeHandoffContext.Provider>);
  expect(html).toContain('Acknowledge unconfirmed handoff');
  expect(html).toContain('Handoff history');
  expect(html).toContain('mesh.recipient_unknown');
});
