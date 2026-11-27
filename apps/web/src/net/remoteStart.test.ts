// ADR-0119: a decision on a start another machine asked for is signed over `agora-approval-start-v4`, with the grant field by field.
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { remoteStartSignedMessage, toHex } from './approval';

interface V4Case {
  name: string; node: string; origin_node: string; origin_agent: string; approval_id: string; approved: boolean;
  grant: { max_running: number; any_kind: boolean; below_folder: boolean; chain: boolean; until_ms: number };
  detail: string; hex: string;
}

describe('a start another machine asked for', () => {
  const vectors = JSON.parse(readFileSync('../../schema/fixtures/signing/approval-start-v4.json', 'utf8')) as { cases: V4Case[] };

  it('signs the vectors the node checks too', async () => {
    expect(vectors.cases.length).toBeGreaterThanOrEqual(5);
    for (const c of vectors.cases) {
      const g = { maxRunning: c.grant.max_running, anyKind: c.grant.any_kind, belowFolder: c.grant.below_folder, chain: c.grant.chain, untilMs: c.grant.until_ms };
      const m = await remoteStartSignedMessage(c.node, c.origin_node, c.origin_agent, c.approval_id, c.approved, g, c.detail);
      expect(toHex(m), c.name).toBe(c.hex);
    }
  });
});
