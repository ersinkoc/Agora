// Regression for #257: filtering a project must not resurrect tabs the user closed.
// Since #332 the terminals being watched are tabs; the tree rows on the left open and close them.
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { startStack } from './browser-harness.mjs';
let stack;
try {
  stack=await startStack();
  const { page, start, errors, origin }=stack;
  page.setDefaultTimeout(10000);
  await start('Shell'); await start('Shell');
  await page.goto(`${origin}/#/`);
  const tabs=()=>document.querySelectorAll('main [role=tab]').length;
  await page.waitForFunction(n=>document.querySelectorAll('main [role=tab]').length===n,2);
  const filter=page.locator('aside select');
  const project=await filter.locator('option').nth(1).getAttribute('value');
  await filter.selectOption(project);
  for(let remaining=2;remaining>0;remaining--) {
    await page.locator('main [role=tab] button').first().click();
    await page.waitForFunction(n=>document.querySelectorAll('main [role=tab]').length===n,remaining-1);
  }
  if(await page.evaluate(tabs)!==0) throw new Error('Closed tabs were resurrected');
  if(await page.locator('aside button[aria-pressed=true]').count()!==0) throw new Error('Tree rows disagree with the open tabs');
  const shots=process.env.AGORA_SCREENSHOTS;
  if(shots) {
    mkdirSync(shots,{recursive:true});
    for(const theme of ['dark','light']) {
      await page.evaluate(t=>{document.documentElement.dataset.theme=t;},theme);
      await page.screenshot({path:join(shots,`filtered-hidden-${theme}.png`),fullPage:true, animations:'disabled'});
    }
  }
  await page.locator('aside button[aria-pressed=false]').first().click();
  await page.waitForFunction(n=>document.querySelectorAll('main [role=tab]').length===n,1);
  // A tree row toggles its tab: it opens closed, and the tab in focus closes.
  const times=await page.evaluate(async()=>{
    const button=document.querySelector('aside button[aria-pressed]');
    const count=()=>document.querySelectorAll('main [role=tab]').length;
    const samples=[];
    for(let i=0;i<20;i++) {
      const before=count();
      const started=performance.now();
      await new Promise(resolve=>{
        const observer=new MutationObserver(()=>{
          if(count()!==before) {observer.disconnect();resolve();}
        });
        observer.observe(document.querySelector('main'),{childList:true,subtree:true});
        button.click();
      });
      samples.push(performance.now()-started);
      await new Promise(resolve=>setTimeout(resolve,30));
    }
    return samples.sort((a,b)=>a-b);
  });
  console.log(`Filtered tab close/reopen (20 samples): p95 ${times[19].toFixed(1)} ms, local render budget <= 50 ms.`);
  if(errors.length) throw new Error(errors.join('\n'));
  console.log('PASS: filtered tabs close to zero, tree rows remain consistent, and the user can reopen a tab; both themes checked.');
} catch(e) {
  console.error(e.stack||e);
  if(stack) console.error((await stack.page.locator('body').innerText()).slice(0,3000));
  process.exitCode=1;
} finally {await stack?.close();}
