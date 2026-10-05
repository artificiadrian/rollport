async page => {
  const out = '/private/tmp/claude-501/-Users-adrian-Desktop-Stuff-dcimport/faa910c0-5587-4d4a-a40f-ad54ebca7e01/scratchpad/site-audit';
  const base = 'http://localhost:4321/rollport';
  const res = [];
  await page.goto(base + '/');
  await page.evaluate(() => sessionStorage.setItem('rollport-dev-switch', JSON.stringify({os:'mac', theme:'auto'})));
  for (const scheme of ['light','dark']) {
    await page.emulateMedia({colorScheme: scheme});
    await page.setViewportSize({width:1280, height:800});
    await page.goto(base + '/help/using');
    await page.addStyleTag({content:'[data-dev-switch],astro-dev-toolbar{display:none!important}'});
    const figs = page.locator('section figure.mt-6');
    const n = await figs.count();
    for (let i=0;i<n;i++){ await figs.nth(i).scrollIntoViewIfNeeded(); await page.waitForTimeout(1500); await figs.nth(i).screenshot({path:`${out}/using-fig${i}-${scheme}.png`}); }
    res.push(scheme+' figs '+n);
    // what is in the done iframe
    res.push(await page.evaluate(() => [...document.querySelectorAll('iframe[data-demo]')].filter(f=>f.getClientRects().length).map(f => f.dataset.demo+': '+(f.contentDocument?.body?.innerText||'').slice(0,80).replace(/\n/g,' | ')).join('\n')));
  }
  return res.join('\n');
}
