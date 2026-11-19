import { describe, expect, it } from 'vitest';
import { hiddenTabs, scrollEdges } from './overflow';

describe('scrollEdges', () => {
  it('has nothing to scroll when the tabs fit', () => {
    expect(scrollEdges({ scrollLeft: 0, clientWidth: 400, scrollWidth: 400 })).toEqual({ overflowing: false, start: false, end: false });
    expect(scrollEdges({ scrollLeft: 0, clientWidth: 400, scrollWidth: 400.5 }).overflowing).toBe(false);
  });
  it('fades the end first, both ends in the middle, the start at the end', () => {
    expect(scrollEdges({ scrollLeft: 0, clientWidth: 300, scrollWidth: 900 })).toEqual({ overflowing: true, start: false, end: true });
    expect(scrollEdges({ scrollLeft: 200, clientWidth: 300, scrollWidth: 900 })).toEqual({ overflowing: true, start: true, end: true });
    expect(scrollEdges({ scrollLeft: 600, clientWidth: 300, scrollWidth: 900 })).toEqual({ overflowing: true, start: true, end: false });
  });
  it('reads a negative scrollLeft (a right-to-left row) by its size', () => {
    expect(scrollEdges({ scrollLeft: -600, clientWidth: 300, scrollWidth: 900 })).toEqual({ overflowing: true, start: true, end: false });
  });
});

describe('hiddenTabs', () => {
  const view = { left: 100, right: 400 };
  it('lists the tabs cut by either edge, in order', () => {
    const tabs = [{ id: 'a', left: 60, right: 160 }, { id: 'b', left: 160, right: 260 }, { id: 'c', left: 260, right: 360 }, { id: 'd', left: 360, right: 460 }, { id: 'e', left: 460, right: 560 }];
    expect(hiddenTabs(view, tabs)).toEqual(['a', 'd', 'e']);
  });
  it('lists none when every tab is inside', () => {
    expect(hiddenTabs(view, [{ id: 'a', left: 100, right: 250 }, { id: 'b', left: 250.5, right: 400.4 }])).toEqual([]);
  });
});
