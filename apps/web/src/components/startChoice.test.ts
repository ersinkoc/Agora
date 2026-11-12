import { describe, expect, it } from 'vitest';
import type { ProjectChoice } from '../net/view';
import { machineFor } from './startChoice';

const project = (node: string): ProjectChoice => ({ node, projectId: `p-${node}`, path: `/work/${node}`, label: node });

describe('machineFor', () => {
  it('keeps the machine already chosen while it is online', () => {
    expect(machineFor('b', ['a', 'b'], 'a', [project('a')])).toBe('b');
  });
  it('picks up a machine that came online after the window opened', () => {
    // Opened with no machine online: nothing was chosen. The node dials in: it is chosen.
    expect(machineFor('', [], undefined, [])).toBe('');
    expect(machineFor('', ['laptop'], undefined, [project('laptop')])).toBe('laptop');
  });
  it('prefers the machine the window was opened for, then one with a known project, then the first', () => {
    expect(machineFor('', ['a', 'b'], 'b', [project('a')])).toBe('b');
    expect(machineFor('', ['a', 'b'], undefined, [project('b')])).toBe('b');
    expect(machineFor('', ['a', 'b'], 'gone', [])).toBe('a');
  });
  it('leaves a machine that went offline for one that is online', () => {
    expect(machineFor('a', ['b'], 'a', [project('a')])).toBe('b');
  });
});
