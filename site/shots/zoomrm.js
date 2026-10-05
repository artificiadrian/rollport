async page => {
  const out = '/private/tmp/claude-501/-Users-adrian-Desktop-Stuff-dcimport/faa910c0-5587-4d4a-a40f-ad54ebca7e01/scratchpad/site-audit';
  const base = 'http://localhost:4321/rollport';
  const res = [];
  const hide = () => page.addStyleTag({content:'[data-dev-switch],astro-dev-toolbar{display:none!important}'});
  await page.emulateMedia({colorScheme:'light', reducedMotion:'no-preference'});
  for (const [w,h,tag] of [[640,400,'200pct'],[427,267,'300pct'],[1280,600,'short']]) {
    await page.setViewportSize({width:w,height:h});
    await page.goto(base+'/'); await hide(); await page.waitForTimeout(2500);
    await page.screenshot({path:`${out}/zoom-home-${tag}.png`});
    res.push(tag+' home scrollH='+await page.evaluate(()=>document.documentElement.scrollHeight)+' vh='+h);
    await page.goto(base+'/help/getting-started'); await hide(); await page.waitForTimeout(500);
    await page.screenshot({path:`${out}/zoom-start-${tag}.png`});
  }
  // reduced motion: does the demo keep changing?
  await page.emulateMedia({reducedMotion:'reduce'});
  await page.setViewportSize({width:1280,height:800});
  await page.goto(base+'/'); await hide();
  const texts=[];
  for (let i=0;i<6;i++){ await page.waitForTimeout(1500); texts.push(await page.frameLocator('iframe[data-demo="live"]').locator('body').innerText().then(t=>t.replace(/\s+/g,' ').slice(0,60))); }
  res.push('reduced motion demo over 9 s:\n  '+texts.join('\n  '));
  await page.screenshot({path:`${out}/reduced-home.png`});
  // ?file= banner, with the download answered as an attachment so the page stays
  await page.emulateMedia({reducedMotion:'no-preference'});
  await page.route('https://github.com/**', r => r.fulfill({status:200, headers:{'content-disposition':'attachment; filename=Rollport.deb','content-type':'application/octet-stream'}, body:'x'}));
  for (const [w, scheme] of [[390,'light'],[1280,'dark']]) {
    await page.emulateMedia({colorScheme:scheme});
    await page.setViewportSize({width:w,height:844});
    await page.goto(base+'/help/getting-started?file=Rollport.deb', {waitUntil:'domcontentloaded'}).catch(e=>res.push('goto: '+e.message.slice(0,80))); await hide(); await page.waitForTimeout(1500);
    res.push('after ?file= url='+page.url()+' os='+await page.evaluate(()=>document.documentElement.dataset.os));
    await page.screenshot({path:`${out}/file-banner-${scheme}-${w}.png`});
  }
  return res.join('\n');
}
