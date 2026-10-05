async page => {
  const out = '/private/tmp/claude-501/-Users-adrian-Desktop-Stuff-dcimport/faa910c0-5587-4d4a-a40f-ad54ebca7e01/scratchpad/site-audit';
  const base = 'http://localhost:4321/rollport';
  const res = [];
  await page.goto(base + '/');
  await page.evaluate(() => sessionStorage.setItem('rollport-dev-switch', JSON.stringify({os:'mac', theme:'auto'})));
  const hide = () => page.addStyleTag({content:'[data-dev-switch]{display:none!important} astro-dev-toolbar{display:none!important}'});
  for (const scheme of ['light','dark']) {
    await page.emulateMedia({colorScheme: scheme});
    await page.setViewportSize({width:1280, height:800});
    // troubleshooting opened
    await page.goto(base + '/help/troubleshooting#not-enough-space'); await hide(); await page.waitForTimeout(500);
    await page.screenshot({path:`${out}/trouble-open-${scheme}-1280.png`});
    const q = page.locator('#windows-heic summary');
    await q.click(); await page.waitForTimeout(400);
    res.push(scheme+' after click hash='+await page.evaluate(()=>location.hash)+' open count='+await page.evaluate(()=>document.querySelectorAll('details[open]').length));
    await page.locator('#windows-heic').screenshot({path:`${out}/trouble-heic-${scheme}.png`});
    await page.locator('#windows-heic summary').hover(); await page.waitForTimeout(200);
    await page.locator('#windows-heic summary').screenshot({path:`${out}/trouble-summary-hover-${scheme}.png`});
    // download page soft button hover + press
    await page.goto(base + '/download'); await hide(); await page.waitForTimeout(300);
    const soft = page.locator('main section a.button').first();
    await soft.hover(); await page.waitForTimeout(250);
    await page.locator('main section').screenshot({path:`${out}/download-soft-hover-${scheme}.png`});
    await page.mouse.down(); await page.waitForTimeout(100);
    await soft.screenshot({path:`${out}/download-soft-press-${scheme}.png`});
    await page.mouse.up();
    const solid = page.locator('main a.button').first();
    await solid.hover(); await page.waitForTimeout(250);
    await solid.screenshot({path:`${out}/download-solid-hover-${scheme}.png`});
    // keyboard focus
    await page.goto(base + '/download'); await hide();
    for (let i=0;i<5;i++) await page.keyboard.press('Tab');
    res.push(scheme+' focused: '+await page.evaluate(()=>document.activeElement.outerHTML.slice(0,80)));
    await page.screenshot({path:`${out}/download-focus-${scheme}.png`, clip:{x:300,y:300,width:680,height:180}});
    // getting started tabs focus & keyboard
    await page.goto(base + '/help/getting-started'); await hide();
    const radios = page.locator('fieldset');
    await page.locator('input[value="mac"]').focus(); await page.keyboard.press('ArrowRight'); await page.waitForTimeout(200);
    res.push(scheme+' os after arrow: '+await page.evaluate(()=>document.documentElement.dataset.os));
    await radios.screenshot({path:`${out}/start-osswitch-focus-${scheme}.png`});
  }
  // format tabs keyboard
  await page.goto(base + '/help/getting-started'); await hide();
  await page.locator('input[value="linux"]').check({force:true}); await page.waitForTimeout(200);
  const tab = page.locator('[data-tab="Rollport.AppImage"]');
  await tab.focus(); await page.keyboard.press('ArrowRight'); await page.waitForTimeout(200);
  res.push('format tab after ArrowRight: selected='+await page.evaluate(()=>document.querySelector('[role=tab][aria-selected=true]:not([hidden])')?.textContent.trim())+' focus='+await page.evaluate(()=>document.activeElement.textContent.trim()));
  await page.keyboard.press('Tab'); 
  res.push('after Tab focus='+await page.evaluate(()=>document.activeElement.textContent.trim().slice(0,30)));
  await page.locator('[data-formats]').first().screenshot({path:`${out}/start-formattabs-focus.png`}).catch(()=>{});
  return res.join('\n');
}
