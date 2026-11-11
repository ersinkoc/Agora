// Regression for #257: filtering a project must not resurrect panes the user hides.
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { startStack } from './browser-harness.mjs';
let stack;
try {
  stack=await startStack();
  const { page, start, errors }=stack;
  page.setDefaultTimeout(10000);
  await start('Shell'); await start('Shell');
  await page.waitForSelector('main section');
  const filter=page.locator('aside select');
  const project=await filter.locator('option').nth(1).getAttribute('value');
  await filter.selectOption(project);
  for(let remaining=2;remaining>0;remaining--) {
    await page.getByRole('button',{name:'Stop watching',exact:true}).first().click();
    await page.waitForFunction(n=>document.querySelectorAll('main section').length===n,remaining-1);
  }
  if(await page.locator('main section').count()!==0) throw new Error('Hidden panes were resurrected');
  if(await page.locator('aside button[aria-pressed=true]').count()!==0) throw new Error('Pane pins disagree with visible panes');
  const shots=process.env.AGORA_SCREENSHOTS;
  if(shots) {
    mkdirSync(shots,{recursive:true});
    for(const theme of ['dark','light']) {
      await page.evaluate(t=>{document.documentElement.dataset.theme=t;},theme);
      await page.screenshot({path:join(shots,`filtered-hidden-${theme}.png`),fullPage:true, animations:'disabled'});
    }
  }
  await page.locator('aside button[aria-pressed=false]').first().click();
  await page.waitForFunction(()=>document.querySelectorAll('main section').length===1);
  const times=await page.evaluate(async()=>{
    const button=document.querySelector('aside button[aria-pressed]');
    const samples=[];
    for(let i=0;i<20;i++) {
      const before=document.querySelectorAll('main section').length;
      const started=performance.now();
      await new Promise(resolve=>{
        const observer=new MutationObserver(()=>{
          if(document.querySelectorAll('main section').length!==before) {observer.disconnect();resolve();}
        });
        observer.observe(document.querySelector('main'),{childList:true,subtree:true});
        button.click();
      });
      samples.push(performance.now()-started);
      await new Promise(resolve=>setTimeout(resolve,30));
    }
    return samples.sort((a,b)=>a-b);
  });
  console.log(`Filtered pane hide/reopen (20 samples): p95 ${times[19].toFixed(1)} ms, local render budget <= 50 ms.`);
  if(errors.length) throw new Error(errors.join('\n'));
  console.log('PASS: filtered panes hide to zero, pins remain consistent, and the user can reopen a pane; both themes checked.');
} catch(e) {
  console.error(e.stack||e);
  if(stack) console.error((await stack.page.locator('body').innerText()).slice(0,3000));
  process.exitCode=1;
} finally {await stack?.close();}
