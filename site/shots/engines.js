async page => {
  const out = '/private/tmp/claude-501/-Users-adrian-Desktop-Stuff-dcimport/faa910c0-5587-4d4a-a40f-ad54ebca7e01/scratchpad/site-audit';
  const base = 'http://localhost:4321/rollport';
  const eng = page.context().browser().browserType().name();
  const logs = [];
  page.on('console', m => { if (m.type()==='error'||m.type()==='warning') logs.push(`${page.url()} ${m.type()}: ${m.text().slice(0,200)}`) });
  page.on('pageerror', e => logs.push(`${page.url()} pageerror: ${e.message}`));
  for (const scheme of ['light','dark']) {
    await page.emulateMedia({colorScheme: scheme});
    for (const [w,h] of [[1280,800],[390,844]]) {
      await page.setViewportSize({width:w,height:h});
      for (const [n,p] of [['home','/'],['download','/download'],['start','/help/getting-started'],['trouble','/help/troubleshooting#trust-refused']]) {
        await page.goto(base+p);
        await page.addStyleTag({content:'[data-dev-switch],astro-dev-toolbar{display:none!important}'});
        await page.waitForTimeout(n==='home'?3000:800);
        await page.screenshot({path:`${out}/${eng}-${n}-${scheme}-${w}.png`, fullPage: n==='download'});
      }
    }
  }
  return eng + '\n' + (logs.join('\n') || 'no console errors');
}
