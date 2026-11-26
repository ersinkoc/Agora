// ADR-0094 step 4: "Claude Code in Shell". The window keeps what its machine found running inside a Shell (`AgentRunsInside`, the
// snapshot's `runs_inside`) while the Shell runs, as the hub does, and names it by the machine's own name for that kind when it has it.
import { describe, expect, it } from 'vitest';
import { applyEvent, emptyView, fromSnapshot, insideKind, kindWords, tiles } from './view';

const started = (v: ReturnType<typeof emptyView>, pos: number) => applyEvent(v, pos, { producerId: 'n', body: 'agentStarted', agentStarted: { agentId: 'a', nodeId: 'n', kind: 'shell' } });

describe('what runs inside a Shell', () => {
  it('is kept while the Shell runs, as a plain kind id, and cleared with its process', () => {
    const v = emptyView();
    started(v, 1);
    let pos = 2;
    const inside = (kind: string) => applyEvent(v, pos++, { producerId: 'n', body: 'agentRunsInside', agentRunsInside: { agentId: 'a', kind } });
    inside('claude-code');
    expect(tiles(v)[0]?.runsInside).toEqual({ kind: 'claude-code', name: 'Claude Code' });
    // Not a plain kind id: left out, the mark stays.
    for (const bad of ['Claude', 'claude code', 'x\u001b[31m', 'k'.repeat(65)]) {
      inside(bad);
      expect(tiles(v)[0]?.runsInside?.kind, bad).toBe('claude-code');
    }
    inside(''); // the program ended
    expect(tiles(v)[0]?.runsInside).toBeUndefined();
    inside('codex');
    started(v, pos++); // a new process starts with none
    expect(tiles(v)[0]?.runsInside).toBeUndefined();
    inside('codex');
    applyEvent(v, pos++, { producerId: 'n', body: 'agentExited', agentExited: { agentId: 'a', exitCode: 0 } });
    expect(tiles(v)[0]?.runsInside).toBeUndefined();
    inside('codex'); // an ended Shell runs nothing
    expect(tiles(v)[0]?.runsInside).toBeUndefined();
    started(v, pos++);
    inside('codex');
    applyEvent(v, pos++, { producerId: 'n', body: 'nodeOnline', nodeOnline: { nodeId: 'n' } });
    expect(tiles(v)[0]?.runsInside).toBeUndefined();
  });

  it('comes from a snapshot for a running Shell only, and takes the machine’s name for the kind when the window has it', () => {
    const s = fromSnapshot({ agents: [
      { agentId: 'n/a', nodeId: 'n', kind: 'shell', running: true, runsInside: 'claude-code' },
      { agentId: 'n/b', nodeId: 'n', kind: 'shell', running: false, runsInside: 'codex' },
      { agentId: 'n/c', nodeId: 'n', kind: 'shell', running: true, runsInside: 'Not A Kind' },
    ] });
    const byId = new Map(tiles(s, (node, kind) => (node === 'n' && kind === 'claude-code' ? 'Claude Code (beta)' : undefined)).map((t) => [t.id, t]));
    expect(byId.get('n/a')?.runsInside).toEqual({ kind: 'claude-code', name: 'Claude Code (beta)' });
    expect(byId.get('n/b')?.runsInside).toBeUndefined();
    expect(byId.get('n/c')?.runsInside).toBeUndefined();
  });

  it('checks kind ids and words them', () => {
    expect(insideKind('registry-qwen-code')).toBe(true);
    expect(insideKind('')).toBe(false);
    expect(insideKind('a/b')).toBe(false);
    expect(kindWords('claude-code')).toBe('Claude Code');
    expect(kindWords('opencode_acp')).toBe('Opencode Acp');
    expect(kindWords('codex')).toBe('Codex');
  });
});
