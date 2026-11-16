// Feed the actual captured protocol frames to the unchanged browser reducer.
import assert from 'node:assert/strict';
import { readFile, writeFile, mkdtemp, rm } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { resolve, join } from 'node:path';
import { pathToFileURL } from 'node:url';

const frames = resolve(process.argv[2]);
const root = process.cwd();
const work = await mkdtemp(resolve('target/0629-web-'));
try {
  const generated = pathToFileURL(resolve('apps/web/src/gen/agora.js')).href;
  let view = stripTypeScriptTypes(await readFile('apps/web/src/net/view.ts', 'utf8'));
  // Module resolution only; reducer statements are left unchanged.
  view = view.replace("'../gen/agora.js'", JSON.stringify(generated)).replace("'../model'", "'./model.mjs'");
  await writeFile(join(work, 'view.mjs'), view);
  await writeFile(join(work, 'model.mjs'), stripTypeScriptTypes(await readFile('apps/web/src/model.ts', 'utf8')));
  const { agora: { v1 } } = await import(generated);
  const { fromSnapshot, applyEvent } = await import(pathToFileURL(join(work, 'view.mjs')).href);
  const fresh = fromSnapshot(v1.Snapshot.decode(await readFile(join(frames, 'snapshot.bin'))));
  const resumed = fromSnapshot(v1.Snapshot.decode(await readFile(join(frames, 'prefix.bin'))));
  const count = Number(await readFile(join(frames, 'count.txt'), 'utf8'));
  for (let i = 0; i < count; i++) {
    const p = v1.PositionedEvent.decode(await readFile(join(frames, `frame-${i}.bin`)));
    applyEvent(resumed, Number(p.pos), p.event);
  }
  console.log(JSON.stringify({
    fresh_agent_visible: fresh.agents.has('n1/a'), replay_agent_visible: resumed.agents.has('n1/a'),
    fresh_victim_count: fresh.nodes.get('n1').agentsRunning, replay_victim_count: resumed.nodes.get('n1').agentsRunning,
    fresh_checkout_node: fresh.checkouts.get('n2/forged').nodeId, replay_checkout_node: resumed.checkouts.get('n2/forged').nodeId,
    fresh_pos: fresh.pos, replay_pos: resumed.pos,
  }, null, 2));
  assert.equal(resumed.agents.has('n1/a'), fresh.agents.has('n1/a'), 'reconnect hides the other machine agent that a fresh snapshot retains');
} finally {
  await rm(work, { recursive: true, force: true });
}
