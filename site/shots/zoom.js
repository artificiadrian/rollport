async page => {
  const out = '/private/tmp/claude-501/-Users-adrian-Desktop-Stuff-dcimport/faa910c0-5587-4d4a-a40f-ad54ebca7e01/scratchpad/site-audit';
  const base = 'http://localhost:4321/rollport';
  await page.setViewportSize({width:1280,height:800});
  for (const scheme of ['light','dark']) {
    await page.emulateMedia({colorScheme: scheme});
    await page.goto(base + '/nope');
    await page.locator('main svg').screenshot({path:`${out}/z-404-art-${scheme}.png`, scale:'device'});
    await page.goto(base + '/download');
    await page.locator('header a').first().screenshot({path:`${out}/z-header-mark-${scheme}.png`});
  }
  // mark at 4x
  await page.emulateMedia({colorScheme:'dark'});
  return 'ok';
}
