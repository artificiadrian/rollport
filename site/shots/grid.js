async page => {
  const out = '/private/tmp/claude-501/-Users-adrian-Desktop-Stuff-dcimport/faa910c0-5587-4d4a-a40f-ad54ebca7e01/scratchpad/site-audit';
  const base = 'http://localhost:4321/rollport';
  const pages = [['home','/'],['download','/download'],['404','/nope'],['help','/help'],['start','/help/getting-started'],['using','/help/using'],['trouble','/help/troubleshooting']];
  const sizes = [[1280,800],[800,1024],[390,844],[360,740]];
  const logs = [];
  page.on('console', m => { if (m.type()==='error'||m.type()==='warning') logs.push(`${page.url()} ${m.type()}: ${m.text()}`) });
  page.on('pageerror', e => logs.push(`${page.url()} pageerror: ${e.message}`));
  for (const os of ['mac','windows','linux']) {
    await page.goto(base + '/');
    await page.evaluate(os => sessionStorage.setItem('rollport-dev-switch', JSON.stringify({os, theme:'auto'})), os);
    for (const scheme of ['light','dark']) {
      await page.emulateMedia({colorScheme: scheme});
      for (const [name, path] of pages) {
        // only home, download, start change with os; others once (mac)
        if (os !== 'mac' && !['home','download','start','trouble'].includes(name)) continue;
        for (const [w,h] of sizes) {
          if (os !== 'mac' && w === 800) continue;
          await page.setViewportSize({width:w, height:h});
          await page.goto(base + path, {waitUntil:'load'});
          await page.addStyleTag({content:'[data-dev-switch]{display:none!important}'});
          await page.waitForTimeout(name==='home'?2500:700);
          await page.screenshot({path:`${out}/${name}-${os}-${scheme}-${w}.png`, fullPage: name!=='home'});
        }
      }
    }
  }
  return logs.join('\n') || 'no console errors';
}
