import { afterEach, describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { setLanguage } from '../i18n';
import type { MachineModel } from '../net/view';
import { NetworkView } from './NetworkView';

const machine = (id: string, os: string): MachineModel => ({ id, name: id, os, online: true, agentsRunning: 0, lastSeenMs: Date.now() });
const chips = (machines: MachineModel[]) => {
  const html = renderToStaticMarkup(<NetworkView machines={machines} agents={[]} hubName="hub" />);
  return [...html.matchAll(/<span class="ide-chip"><bdi>([^<]*)<\/bdi><\/span>/g)].map((m) => m[1]);
};

describe('Your machines: the system (#944, RM-03)', () => {
  afterEach(() => setLanguage('en'));

  it('names each system through the os-* messages, not as the node reports it', () => {
    setLanguage('en');
    expect(chips([machine('a', 'windows'), machine('b', 'macos'), machine('c', 'linux')])).toEqual(['Windows', 'macOS', 'Linux']);
    setLanguage('tr');
    expect(chips([machine('a', 'windows'), machine('b', 'macos'), machine('c', 'linux')])).toEqual(['Windows', 'macOS', 'Linux']);
  });

  it('shows a system it has no words for as reported, and an unknown one as a question mark', () => {
    setLanguage('en');
    expect(chips([machine('a', 'freebsd'), machine('b', '')])).toEqual(['freebsd', '?']);
    // What a node sends is made safe to show: no control or reordering characters.
    expect(chips([machine('a', 'free‮bsd\u0007')])).toEqual(['free bsd']);
  });
});
