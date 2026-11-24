import { describe, expect, it } from 'vitest';
import type { ProjectChoice } from '../net/view';
import { isBareName, machineFor, missingNote, shortBinary, trustCommand } from './startChoice';

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
describe('missingNote', () => {
  it('says why only for a kind the machine cannot find: its program by name, or the general sentence when the machine names none', () => {
    expect(missingNote({ installed: false, program: 'codex' })).toEqual(['start-not-installed-program', { program: 'codex' }]);
    expect(missingNote({ installed: false })).toEqual(['start-not-installed-why']);
    expect(missingNote({ installed: true, program: 'claude' })).toBeUndefined();
    expect(missingNote(undefined)).toBeUndefined();
  });
});

describe('shortBinary and trustCommand', () => {
  it('shows only the last two parts of the program path, never the user folder', () => {
    expect(shortBinary(String.raw`C:\Users\someone\Downloads\agora.exe`)).toBe(String.raw`…\Downloads\agora.exe`);
    expect(shortBinary('/home/someone/bin/agora')).toBe('…/bin/agora');
    expect(shortBinary(String.raw`Downloads\agora.exe`)).toBe(String.raw`Downloads\agora.exe`);
  });
  it('runs the program from its own folder when it is off the PATH, and by name otherwise', () => {
    expect(trustCommand(undefined, 'windows')).toBe('agora node trust --from-hub');
    expect(trustCommand(String.raw`C:\Users\someone\Downloads\agora.exe`, 'windows')).toBe(String.raw`.\agora.exe node trust --from-hub`);
    expect(trustCommand('/home/someone/bin/agora', 'linux')).toBe('./agora node trust --from-hub');
  });
  it('quotes a file name the shell would split, in the form each shell takes (#599 review)', () => {
    // A second download into Downloads: PowerShell needs the call operator around a quoted path.
    expect(trustCommand(String.raw`C:\Users\x\Downloads\agora (1).exe`, 'windows')).toBe(String.raw`& '.\agora (1).exe' node trust --from-hub`);
    expect(trustCommand(String.raw`C:\x\it's agora.exe`, 'windows')).toBe(String.raw`& '.\it''s agora.exe' node trust --from-hub`);
    expect(trustCommand('/home/x/agora (1)', 'macos')).toBe(`'./agora (1)' node trust --from-hub`);
    expect(trustCommand("/home/x/it's", 'linux')).toBe(String.raw`'./it'\''s' node trust --from-hub`);
  });
  it('treats a file name with no folder as bare, as a member receives it from the hub (#598)', () => {
    expect(isBareName('agora.exe')).toBe(true);
    expect(isBareName('agora (1).exe')).toBe(true);
    expect(isBareName(String.raw`Downloads\agora.exe`)).toBe(false);
    expect(isBareName('bin/agora')).toBe(false);
    // A bare name is shown as it is (nothing to shorten, nothing more to reveal) and run from its folder.
    expect(shortBinary('agora.exe', 'windows')).toBe('agora.exe');
    expect(trustCommand('agora (1).exe', 'windows')).toBe(String.raw`& '.\agora (1).exe' node trust --from-hub`);
  });
  it('tells Windows from the machine, not from a backslash, so a bare file name still gets the Windows form (#599 review)', () => {
    expect(trustCommand('agora.exe', 'windows')).toBe(String.raw`.\agora.exe node trust --from-hub`);
    expect(shortBinary(String.raw`Users\x\Downloads\agora.exe`, 'windows')).toBe(String.raw`…\Downloads\agora.exe`);
    expect(shortBinary('home/x/bin/agora', 'linux')).toBe('…/bin/agora');
  });
});
