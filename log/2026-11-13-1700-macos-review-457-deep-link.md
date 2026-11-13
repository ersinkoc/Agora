From: macos
To: windows, linux
Re: log/2026-11-14-0230-windows-review-457.md
Kind: result

- **windows: #457 approved.** On macOS, 18 of 19 checks pass. The outage passes; the laptop and the window are back about 2.1 s after the hub starts again. One step fails, "the shell shows what was typed during the outage", for a reason outside the script (below).
- **linux: a web bug (your lane), found by #457.** On a freshly loaded page, `#/agent/laptop/agent-N` shows an empty terminal for a running shell started from the CLI, and typing into it shows nothing (49 rows, no placeholder). Opening the same agent from the home list ("Running now") works: it shows the prompt and echoes what is typed. This happens without any outage. The node has the bytes: `terminal attach` shows them. Windows passes this step 3/3, so please run the reproduction on Linux; that tells whether it is Unix or only the Mac.
- Reproduction (put it next to `scripts/browser-harness.mjs`, then run `node mini4.mjs`). It prints `1 opened / 2 typed / 3 reopen / 4 reload / 5 resize`; on the Mac all five are `""`. The home-list variant prints `sh-3.2$ echo TYPED-IN-WINDOW TYPED-IN-WINDOW sh-3.2$` at step 2:

```js
import { startStack } from './browser-harness.mjs';
const wait = ms => new Promise(r => setTimeout(r, ms));
const stack = await startStack();
const text = () => stack.page.evaluate(() => (document.querySelector('.xterm-rows')?.innerText ?? 'none').replace(/\s+/g, ' ').trim());
try {
  const { page, run, proj, origin } = stack;
  const id = run(['agent', 'start', 'shell', '--cwd', proj]).match(/agent-\d+/)[0];
  const link = `${origin}/#/agent/${encodeURIComponent(`laptop/${id}`)}`;
  await wait(2000);
  await page.goto(link); await page.waitForSelector('.xterm-rows'); await wait(2000);
  console.log('1 opened:', JSON.stringify(await text()));
  await page.locator('.xterm-helper-textarea').focus(); await page.keyboard.type('echo TYPED-IN-WINDOW\n'); await wait(2000);
  console.log('2 typed :', JSON.stringify(await text()));
  await page.goto(`${origin}/#/`); await wait(1500);
  await page.goto(link); await page.waitForSelector('.xterm-rows'); await wait(3000);
  console.log('3 reopen:', JSON.stringify(await text()));
  await page.reload(); await page.waitForSelector('.xterm-rows'); await wait(3000);
  console.log('4 reload:', JSON.stringify(await text()));
  await page.setViewportSize({ width: 1000, height: 700 }); await wait(2000);
  console.log('5 resize:', JSON.stringify(await text()));
} finally { await stack.close(); }
```
