async page => {
  const out = '/private/tmp/claude-501/-Users-adrian-Desktop-Stuff-dcimport/faa910c0-5587-4d4a-a40f-ad54ebca7e01/scratchpad/site-audit';
  const base = 'http://localhost:4321/rollport';
  const hide = async () => page.addStyleTag({content:'[data-dev-switch],astro-dev-toolbar{display:none!important}'});
  const res = [];
  for (const [os, w, scheme] of [['windows',390,'dark'],['linux',360,'light'],['linux',1280,'dark'],['windows',1280,'light']]) {
    await page.goto(base + '/');
    await page.evaluate(os => sessionStorage.setItem('rollport-dev-switch', JSON.stringify({os, theme:'auto'})), os);
    await page.emulateMedia({colorScheme: scheme});
    await page.setViewportSize({width:w, height:900});
    await page.goto(base + '/help/getting-started'); await hide(); await page.waitForTimeout(500);
    for (const id of (os==='windows'?['apple-devices','install']:['usbmuxd','install'])) {
      await page.locator('#'+id).screenshot({path:`${out}/start-${id}-${os}-${scheme}-${w}.png`});
    }
    res.push(os+w+' scrollWidth '+await page.evaluate(()=>document.documentElement.scrollWidth));
  }
  // ?file= flow: block the github navigation
  await page.route('https://github.com/**', r => r.fulfill({status:404, body:'github 404 (blocked in audit)'}));
  await page.emulateMedia({colorScheme:'light'});
  await page.setViewportSize({width:390,height:844});
  const nav = [];
  page.on('framenavigated', f => { if (f === page.mainFrame()) nav.push(f.url()) });
  await page.goto(base + '/help/getting-started?file=Rollport.deb', {waitUntil:'commit'});
  await page.waitForTimeout(1500);
  res.push('nav after ?file=: ' + nav.join(' -> '));
  return res.join('\n');
}
