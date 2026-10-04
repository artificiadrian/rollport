// Takes the link preview image (og:image) from the top of the site: the
// headline beside the live app, as a visitor sees it first.
// With the site's dev server running: pnpm capture
import { chromium } from "playwright"
import { fileURLToPath } from "node:url"

const SITE = process.env["SITE"] ?? "http://localhost:4321"

const browser = await chromium.launch()
const page = await browser.newPage({
    viewport: { width: 1200, height: 630 },
    deviceScaleFactor: 1,
    colorScheme: "light",
    // The demo starts the import by itself; stay on ready for the picture.
    reducedMotion: "reduce",
})
page.on("pageerror", (e) => console.log("pageerror", e.message))
await page.goto(SITE, { waitUntil: "networkidle" })
// Astro's dev toolbar is not part of the site.
await page.addStyleTag({ content: "astro-dev-toolbar { display: none }" })
await page.waitForTimeout(1500)
await page.screenshot({
    path: fileURLToPath(new URL("../public/og.png", import.meta.url)),
})
await browser.close()
console.log("capture: public/og.png")
