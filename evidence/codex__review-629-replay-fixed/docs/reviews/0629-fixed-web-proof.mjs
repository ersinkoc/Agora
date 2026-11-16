// Actual browser reducer, with import resolution rewritten only in a temporary module.
import assert from 'node:assert/strict';
import { readFile, writeFile, mkdtemp, rm } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { resolve, join } from 'node:path';
import { pathToFileURL } from 'node:url';

const frames = resolve(process.argv[2]);
const work = await mkdtemp(resolve('target/0629-fixed-web-'));
try {
  const generated = pathToFileURL(resolve('apps/web/src/gen/agora.js')).href;
  let view = stripTypeScriptTypes(await readFile('apps/web/src/net/view.ts', 'utf8'));
  view = view.replace("'../gen/agora.js'", JSON.stringify(generated)).replace("'../model'", "'./model.mjs'");
  await writeFile(join(work, 'view.mjs'), view);
  await writeFile(join(work, 'model.mjs'), stripTypeScriptTypes(await readFile('apps/web/src/model.ts', 'utf8')));
  const { agora: { v1 } } = await import(generated);
  const { fromSnapshot, applyEvent } = await import(pathToFileURL(join(work, 'view.mjs')).href);
  const fresh = fromSnapshot(v1.Snapshot.decode(await readFile(join(frames, 'fresh.bin'))));
  let dirty = fromSnapshot(v1.Snapshot.decode(await readFile(join(frames, 'prefix.bin'))));
  for (let i = 0; i < 5; i++) {
    const p = v1.PositionedEvent.decode(await readFile(join(frames, `legacy-${i}.bin`)));
    applyEvent(dirty, Number(p.pos), p.event);
  }
  const before = {
    pos: dirty.pos, agent: dirty.agents.has('n1/a'),
    count: dirty.nodes.get('n1').agentsRunning, checkout: dirty.checkouts.get('n2/forged').nodeId,
  };
  assert.equal(before.pos, 8);
  assert.equal(before.agent, false);
  assert.equal(before.count, 99);
  assert.equal(before.checkout, 'n1');
  // The reconnect's actual clean snapshot replaces the already-corrupt client state.
  dirty = fromSnapshot(v1.Snapshot.decode(await readFile(join(frames, 'resumed.bin'))));
  assert.deepEqual(dirty, fresh);
  console.log(JSON.stringify({ before, healed: { pos: dirty.pos, agent: dirty.agents.has('n1/a'), count: dirty.nodes.get('n1').agentsRunning, checkout: dirty.checkouts.get('n2/forged').nodeId }, equal_to_fresh: true }, null, 2));
} finally {
  await rm(work, { recursive: true, force: true });
}
