// Builds the app's interface into public/demo/ with the fake Rust side from
// demo/tauri.ts, so the site shows the real window as the app has it now.
import { execFileSync } from "node:child_process"
import { cp, readFile, rm, writeFile } from "node:fs/promises"
import { stripTypeScriptTypes } from "node:module"
import { fileURLToPath } from "node:url"
import { chromium } from "playwright"
import { sentence, site, url } from "../src/site.ts"

const app = new URL("../../", import.meta.url)
const out = new URL("../public/demo/", import.meta.url)

execFileSync("pnpm", ["build"], { cwd: fileURLToPath(app), stdio: "inherit" })

await rm(out, { recursive: true, force: true })
await cp(new URL("build/", app), out, { recursive: true })
await cp(new URL("../demo/photos/", import.meta.url), new URL("photos/", out), {
    recursive: true,
})

const page = new URL("index.html", out)
const fake = stripTypeScriptTypes(
    await readFile(new URL("../demo/tauri.ts", import.meta.url), "utf8"),
)
const built = await readFile(page, "utf8")
// The app loads its code from /_app/, and here that is /demo/_app/ under the
// site's base. Its base path it works out from the page's own address (hash
// router).
if (!built.includes('"/_app/'))
    throw new Error(
        "the app's index.html no longer loads /_app/; update scripts/demo.ts",
    )
const html = built
    .replaceAll('"/_app/', `"${url("/demo/_app/")}`)
    // A frame shows no icon, and the app's icon path has no base.
    .replace(/<link rel="icon"[^>]*>\s*/, "")
    // Before the app's own scripts, which reach for the Tauri globals at once.
    .replace("<head>", () => `<head>\n<script>\n${fake}</script>`)
await writeFile(page, html)
console.log("demo: built into public/demo/")

// The help page's pictures: the app played to each screen here, at build time,
// and saved as it then stands, without its code. Visitors get them drawn at
// once. "options" is the ready screen with Options opened by a click. One set
// per system (?os=, see demo/tauri.ts): the app words some screens by system.
const STILLS = [
    "trust",
    "folder",
    "ready",
    "options",
    "importing",
    "done",
] as const
const SYSTEMS = ["mac", "windows", "linux"] as const
const browser = await chromium.launch()
const context = await browser.newContext({
    viewport: { width: 400, height: 560 },
})
// Served from public/ on disk, as the site will serve it, under its base.
await context.route("http://site.local/**", (route) =>
    route.fulfill({
        path: fileURLToPath(
            new URL(
                `..${new URL(route.request().url()).pathname.slice(site.base.length)}`,
                out,
            ),
        ),
    }),
)
const heights: Record<string, number> = {}
for (const os of SYSTEMS)
    for (const still of STILLS) {
        const tab = await context.newPage()
        tab.on("pageerror", (error) => {
            throw error
        })
        const state = still === "options" ? "ready" : still
        await tab.goto(
            `http://site.local${url("/demo/index.html")}?state=${state}&os=${os}`,
        )
        await tab.waitForFunction(() => "__settled" in window)
        if (still === "options") {
            await tab.getByRole("button", { name: "Options" }).click()
            await tab.waitForTimeout(500)
        }
        const saved = await tab.evaluate(() => {
            // Only app.html's theme script stays: light and dark follow the system.
            for (const script of document.querySelectorAll("script"))
                if (!script.textContent.includes("prefers-color-scheme"))
                    script.remove()
            // The icon's address is the build browser's, and a frame shows no icon.
            for (const link of document.querySelectorAll(
                'link[rel="modulepreload"], link[rel="icon"]',
            ))
                link.remove()
            document.documentElement.classList.remove("dark")
            return {
                html: `<!doctype html>\n${document.documentElement.outerHTML}`,
                height: document.body.scrollHeight,
            }
        })
        heights[`${still}-${os}`] = saved.height
        await writeFile(new URL(`${still}-${os}.html`, out), saved.html)
        await tab.close()
    }
await writeFile(new URL("stills.json", out), JSON.stringify(heights))
console.log("demo: stills saved", heights)

// The live demo draws the ready still at once, over the app, while the app's
// code loads; demo/tauri.ts takes it away once the app has drawn the same
// screen. The systems' ready screens differ only in a tooltip.
const readyStill = await readFile(new URL("ready-mac.html", out), "utf8")
const readyBody = /<body>([\s\S]*)<\/body>/.exec(readyStill)?.[1]
if (!readyBody) throw new Error("the ready still has no <body>")
// The still fades in with its photos, not before: until they arrive its
// prints are blank paper, the app's look for a missing thumbnail. After 2 s
// it shows as it is. demo/tauri.ts waits for __stillShown before the app
// takes over, so the app's own blank prints do not show either.
const showStill = `{
    const still = document.querySelector("[data-still]")
    const photos = [...still.querySelectorAll("img")].map((image) => image.decode().catch(() => {}))
    window.__stillShown = Promise.race([
        Promise.all(photos),
        new Promise((done) => setTimeout(done, 2000)),
    ]).then(() => {
        still.style.opacity = "1"
    })
}`
await writeFile(
    page,
    html.replace(
        "<body>",
        () =>
            `<body>\n<div data-first class="fixed inset-0 z-50 bg-canvas"><div data-still style="opacity: 0; transition: opacity 300ms">${readyBody}</div></div>\n<script>${showStill}</script>`,
    ),
)

// The share image for link previews (og:image, 1200 x 630) and the home-screen
// icon (apple-touch-icon, 180 x 180): drawn from the site's own parts, the
// logo, the sentence, and the ready screen just saved, so they never go stale.
const publicDir = new URL("../public/", import.meta.url)
await context.route("http://font.local/**", (route) =>
    route.fulfill({
        path: fileURLToPath(
            import.meta
                .resolve("@fontsource-variable/hanken-grotesk/files/hanken-grotesk-latin-wght-normal.woff2"),
        ),
    }),
)
// The logo (src/components/Mark.astro), as the app icon draws it.
const logo = (size: number) => `
    <svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="#21201c" stroke-width="1.5" stroke-linejoin="round" stroke-linecap="round">
        <rect x="2.5" y="6" width="12" height="10" rx="1.6" transform="rotate(-9 8.5 11)" fill="#f9f9f8" stroke="#8d8d86" />
        <rect x="9.5" y="5" width="12" height="10" rx="1.6" transform="rotate(8 15.5 10)" fill="#f9f9f8" stroke="#8d8d86" />
        <rect x="6" y="8" width="12" height="10" rx="1.6" fill="#fdfdfc" />
        <path d="m7.5 16 3-3 2 2 1.5-1.5 2.5 2.5" />
    </svg>`
const share = await context.newPage()
await share.setViewportSize({ width: 1200, height: 630 })
await share.setContent(`<!doctype html>
<style>
    @font-face { font-family: Hanken; src: url(http://font.local/hanken.woff2) format("woff2"); font-weight: 100 900; }
    body { margin: 0; width: 1200px; height: 630px; overflow: hidden; position: relative; background: #fdfdfc; color: #21201c; font-family: Hanken, sans-serif; }
    .text { position: absolute; left: 80px; top: 88px; width: 540px; }
    .name { display: flex; align-items: center; gap: 16px; font-size: 44px; font-weight: 650; }
    p { margin: 44px 0 0; font-size: 34px; line-height: 1.3; text-wrap: pretty; }
    .glow { position: absolute; left: 640px; top: 0; width: 620px; height: 700px; background: url(http://site.local${url("/demo/photos/sunset.svg")}) center / cover; filter: blur(90px) saturate(1.5); opacity: 0.28; }
    .window { position: absolute; left: 720px; top: 72px; width: 400px; border-radius: 12px; overflow: hidden; background: #fdfdfc; box-shadow: 0 0 0 1px rgb(33 32 28 / 0.1), 0 24px 60px rgb(33 32 28 / 0.18); }
    .bar { height: 28px; display: flex; align-items: center; gap: 8px; padding-left: 12px; background: #f1f0ef; border-bottom: 1px solid #e4e2df; }
    .bar i { width: 12px; height: 12px; border-radius: 50%; }
    iframe { display: block; width: 400px; height: ${heights["ready-mac"] ?? 560}px; border: 0; }
</style>
<div class="glow"></div>
<div class="text">
    <div class="name">${logo(52)} Rollport</div>
    <p>${sentence}</p>
</div>
<div class="window">
    <div class="bar"><i style="background:#ff5f57"></i><i style="background:#febc2e"></i><i style="background:#28c840"></i></div>
    <iframe src="http://site.local${url("/demo/ready-mac.html")}"></iframe>
</div>`)
await share.evaluate(() => document.fonts.ready)
await share.waitForTimeout(500)
await share.screenshot({ path: fileURLToPath(new URL("og.png", publicDir)) })

const icon = await context.newPage()
await icon.setViewportSize({ width: 180, height: 180 })
// Full bleed: iOS rounds the corners itself.
await icon.setContent(`<!doctype html>
<style>
    body { margin: 0; width: 180px; height: 180px; display: grid; place-items: center; background: linear-gradient(#fdfdfc, #e9e8e6); }
</style>
${logo(128)}`)
await icon.screenshot({
    path: fileURLToPath(new URL("apple-touch-icon.png", publicDir)),
})
await browser.close()
console.log("demo: share image and touch icon saved")
