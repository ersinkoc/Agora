import { describe, expect, it } from 'vitest';
import type { ProjectChoice } from '../net/view';
import { machineFor, missingPrograms, shortBinary, trustCommand } from './startChoice';

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

// ADR-0069: what the start window says about a missing program, and the trust command when the machine's program is off its PATH.
describe('missingPrograms', () => {
  it('names each missing program once, and asks for the general sentence only for a kind that names none', () => {
    expect(missingPrograms([
      { installed: true, program: 'claude' },
      { installed: false, program: 'codex' },
      { installed: false, program: 'codex' },
      { installed: false, program: 'gemini' },
    ])).toEqual({ programs: ['codex', 'gemini'], general: false });
    expect(missingPrograms([{ installed: false }, { installed: false, program: 'codex' }])).toEqual({ programs: ['codex'], general: true });
    expect(missingPrograms([{ installed: true }])).toEqual({ programs: [], general: false });
  });
});

describe('shortBinary and trustCommand', () => {
  it('shows only the last two parts of the program path, never the user folder', () => {
    expect(shortBinary(String.raw`C:\Users\someone\Downloads\agora.exe`)).toBe(String.raw`…\Downloads\agora.exe`);
    expect(shortBinary('/home/someone/bin/agora')).toBe('…/bin/agora');
    expect(shortBinary(String.raw`Downloads\agora.exe`)).toBe(String.raw`Downloads\agora.exe`);
  });
  it('runs the program from its own folder when it is off the PATH, and by name otherwise', () => {
    expect(trustCommand(undefined, 'windows', 'd1', 'k1')).toBe('agora node trust d1 k1');
    expect(trustCommand(String.raw`C:\Users\someone\Downloads\agora.exe`, 'windows', 'd1', 'k1')).toBe(String.raw`.\agora.exe node trust d1 k1`);
    expect(trustCommand('/home/someone/bin/agora', 'linux', 'd1', 'k1')).toBe('./agora node trust d1 k1');
  });
  it('quotes a file name the shell would split, in the form each shell takes (#599 review)', () => {
    // A second download into Downloads: PowerShell needs the call operator around a quoted path.
    expect(trustCommand(String.raw`C:\Users\x\Downloads\agora (1).exe`, 'windows', 'd1', 'k1')).toBe(String.raw`& '.\agora (1).exe' node trust d1 k1`);
    expect(trustCommand(String.raw`C:\x\it's agora.exe`, 'windows', 'd1', 'k1')).toBe(String.raw`& '.\it''s agora.exe' node trust d1 k1`);
    expect(trustCommand('/home/x/agora (1)', 'macos', 'd1', 'k1')).toBe(`'./agora (1)' node trust d1 k1`);
    expect(trustCommand("/home/x/it's", 'linux', 'd1', 'k1')).toBe(String.raw`'./it'\''s' node trust d1 k1`);
  });
  it('tells Windows from the machine, not from a backslash, so a bare file name still gets the Windows form (#599 review)', () => {
    expect(trustCommand('agora.exe', 'windows', 'd1', 'k1')).toBe(String.raw`.\agora.exe node trust d1 k1`);
    expect(shortBinary(String.raw`Users\x\Downloads\agora.exe`, 'windows')).toBe(String.raw`…\Downloads\agora.exe`);
    expect(shortBinary('home/x/bin/agora', 'linux')).toBe('…/bin/agora');
  });
});
