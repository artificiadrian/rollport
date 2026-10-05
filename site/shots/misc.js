async page => {
  const out = '/private/tmp/claude-501/-Users-adrian-Desktop-Stuff-dcimport/faa910c0-5587-4d4a-a40f-ad54ebca7e01/scratchpad/site-audit';
  const base = 'http://localhost:4321/rollport';
  const res = [];
  await page.emulateMedia({colorScheme: 'light'});
  for (const w of [360,390,800]) {
    await page.setViewportSize({width:w, height:740});
    for (const p of ['/','/download','/help','/help/getting-started','/help/using','/help/troubleshooting','/nope']) {
      await page.goto(base + p);
      const r = await page.evaluate(() => {
        const vw = document.documentElement.clientWidth;
        const over = [...document.querySelectorAll('body *')].filter(e => { const b = e.getBoundingClientRect(); return b.width && (b.right > vw + 1) && !e.closest('nav,[data-dev-switch],astro-dev-toolbar,code,pre') && getComputedStyle(e).position!=='fixed'; }).slice(0,5).map(e => e.tagName+'.'+String(e.className).slice(0,40)+' r='+Math.round(e.getBoundingClientRect().right));
        return document.documentElement.scrollWidth + ' > ' + vw + ' ? ' + over.join(' ; ');
      });
      res.push(w + ' ' + p + ' ' + r);
    }
  }
  await page.setViewportSize({width:360, height:740});
  await page.goto(base + '/help/using#keys');
  await page.addStyleTag({content:'[data-dev-switch]{display:none!important}'});
  await page.locator('#keys').screenshot({path:`${out}/using-keys-360.png`});
  await page.locator('#file-names').screenshot({path:`${out}/using-filenames-360.png`});
  return res.join('\n');
}
