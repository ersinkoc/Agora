import { describe, expect, it } from 'vitest';
import { defaultSort, matches, screenshotAddress, sorted, type Listed } from './order';

const item = (id: string, over: Partial<Listed> = {}): Listed => ({ id, name: id, author: 'a', added: '', popularity: 0, ...over });

describe('the gallery order (ADR-0120)', () => {
  it('opens on popular when the catalog publishes popularity, else new, else by name', () => {
    expect(defaultSort([item('a'), item('b', { popularity: 3 })])).toBe('popular');
    expect(defaultSort([item('a'), item('b', { added: '2026-11-20' })])).toBe('new');
    expect(defaultSort([item('a')])).toBe('name');
  });

  it('orders by popularity, by day (no day last), by name and by author, ties by name', () => {
    const list = [
      item('moss', { name: 'Moss', author: 'zed', popularity: 5, added: '2026-10-01' }),
      item('ember', { name: 'Ember', author: 'amy', popularity: 9 }),
      item('glacier', { name: 'Glacier', author: 'amy', popularity: 5, added: '2026-11-20' }),
    ];
    expect(sorted(list, 'popular').map((i) => i.id)).toEqual(['ember', 'glacier', 'moss']);
    expect(sorted(list, 'new').map((i) => i.id)).toEqual(['glacier', 'moss', 'ember']);
    expect(sorted(list, 'name').map((i) => i.id)).toEqual(['ember', 'glacier', 'moss']);
    expect(sorted(list, 'author').map((i) => i.id)).toEqual(['ember', 'glacier', 'moss']);
    expect(list.map((i) => i.id), 'the list itself is not changed').toEqual(['moss', 'ember', 'glacier']);
  });

  it('finds every typed word in the name, author or description, ignoring case', () => {
    const i = item('harbour', { name: 'Harbour', author: 'Mira', description: 'A calm blue theme' });
    expect(matches(i, '')).toBe(true);
    expect(matches(i, 'harb mira')).toBe(true);
    expect(matches(i, 'BLUE calm')).toBe(true);
    expect(matches(i, 'harbour red')).toBe(false);
  });

  it('shows a screenshot only as the image type its first bytes say', () => {
    const png = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13]);
    const webp = new TextEncoder().encode('RIFF\0\0\0\0WEBPVP8X');
    expect(screenshotAddress('image/png', png)).toMatch(/^data:image\/png;base64,iVBORw0KGgo/);
    expect(screenshotAddress('image/webp', webp)).toMatch(/^data:image\/webp;base64,/);
    expect(screenshotAddress('image/webp', png)).toBeUndefined();
    expect(screenshotAddress('image/png', webp)).toBeUndefined();
    expect(screenshotAddress('image/svg+xml', new TextEncoder().encode('<svg/>'))).toBeUndefined();
    expect(screenshotAddress('text/html', png)).toBeUndefined();
  });
});
