import { describe, expect, it } from 'vitest';
import { applyEvent, fromSnapshot, machines } from './view';

// ADR-0069: a machine's program path when its folder is not on the PATH, from the snapshot and from a start.
describe('a machine whose program is not on its PATH', () => {
  it('keeps the path from the snapshot, and a start replaces it together with the build', () => {
    const path = String.raw`C:\Users\x\Downloads\agora.exe`;
    const view = fromSnapshot({ nodes: [{ nodeId: 'win', hostname: 'WHITE', online: true, lastEventMs: 1, build: '0.3.12', binary: path }] });
    expect(machines(view)[0]).toMatchObject({ build: '0.3.12', binary: path });
    // It starts again from a folder on its PATH: the path is gone, and the new build is shown at once.
    applyEvent(view, 5, { producerId: 'win', body: 'nodeOnline', nodeOnline: { nodeId: 'win', hostname: 'WHITE', os: 'windows', build: '0.3.13', binary: '' } } as never);
    expect(machines(view)[0]).toMatchObject({ build: '0.3.13', binary: '' });
  });
});
