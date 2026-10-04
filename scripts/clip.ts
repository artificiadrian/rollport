// The README's clip: the built app with the site demo's fake Rust side, which
// plays ready, an import and its report by itself. The app sits in a Windows
// 11 window frame with the site's shadow, on a transparent ground, so the clip
// rests on GitHub's light or dark page; it is recorded once for each scheme.
// Frames come from Chromium's screencast as PNGs with their times, and
// img2webp (brew install webp) makes an animated WebP of them: full colour
// and an alpha channel, where a GIF has 256 colours and a one-bit alpha.
//
//     pnpm clip        writes docs/rollport.webp and docs/rollport-dark.webp
import { execFileSync } from "node:child_process"
import { mkdir, readFile, rm, stat, writeFile } from "node:fs/promises"
import { stripTypeScriptTypes } from "node:module"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { fileURLToPath } from "node:url"
import { chromium } from "playwright"

const root = new URL("../", import.meta.url)
const build = fileURLToPath(new URL("build/", root))
const photos = fileURLToPath(new URL("site/demo/photos/", root))
const docs = fileURLToPath(new URL("docs/", root))

/// The demo waits this long on the ready screen before it imports; the clip
/// starts a moment before that and ends once the report has settled.
const IDLE = 5000
const FROM = IDLE - 1500
const UNTIL = IDLE + 6000 + 3000
/// Frames closer together than this are one frame: 20 a second is smooth
/// for prints sliding, and half the size of 40.
const STEP = 50

/// The app's window, the title bar over it, and the room around the frame
/// for its shadow, in CSS pixels. The page is drawn at SCALE, so the clip is
/// sharp on a Retina screen: the screencast gives one pixel per CSS pixel,
/// whatever the device scale factor.
const APP = { width: 400, height: 560 }
const BAR = 32
// The larger shadow reaches about 56 px to the sides and 96 px below: with
// less room it ends in a hard edge at the clip's border.
const ROOM = { top: 48, side: 72, bottom: 112 }
const SCALE = 2
const WIDTH = APP.width + 2 * ROOM.side
const HEIGHT = ROOM.top + BAR + APP.height + ROOM.bottom

execFileSync("pnpm", ["build"], { cwd: fileURLToPath(root), stdio: "inherit" })

const fake = stripTypeScriptTypes(
    await readFile(new URL("site/demo/tauri.ts", root), "utf8"),
)
// In the page's head, as the site's demo has it: the fake watches the
// document, which does not exist yet for a script that runs before the page.
const app = (await readFile(build + "index.html", "utf8")).replace(
    "<head>",
    () => `<head><script>${fake}</script>`,
)

// The frame as site/src/components/Window.astro draws Windows 11: the title on
// the left, minimise, maximise and close on the right, the edge ring and the
// site's shadow. Windows is where the people who need the app most are. The
// app lays out at its own 400 wide in the iframe and is scaled with the page.
const frame = `<!doctype html>
<html>
<head>
<meta charset="utf-8">
<style>
    html, body { margin: 0; background: transparent; }
    .stage {
        width: ${WIDTH}px;
        height: ${HEIGHT}px;
        padding: ${ROOM.top}px ${ROOM.side}px ${ROOM.bottom}px;
        box-sizing: border-box;
        transform: scale(${SCALE});
        transform-origin: 0 0;
    }
    .frame {
        overflow: hidden;
        border-radius: 8px;
        background: #fdfdfc;
        box-shadow:
            0 0 0 1px rgb(33 32 28 / 0.12),
            0 40px 80px -24px rgb(60 30 40 / 0.35),
            0 16px 32px -16px rgb(33 32 28 / 0.2);
    }
    .bar {
        display: flex;
        align-items: center;
        height: ${BAR}px;
        background: #f3f3f3;
        font: 12px "Segoe UI Variable", "Segoe UI", system-ui, sans-serif;
        color: #1b1b1b;
    }
    .bar span { flex: 1; padding: 0 12px; }
    .bar b { display: grid; place-items: center; width: 46px; height: 100%; }
    .bar svg { width: 10px; height: 10px; }
    iframe { display: block; border: 0; width: ${APP.width}px; height: ${APP.height}px; }
    @media (prefers-color-scheme: dark) {
        .frame {
            background: #222221;
            box-shadow:
                0 0 0 1px rgb(255 255 255 / 0.1),
                0 30px 60px -24px rgb(0 0 0 / 0.55),
                0 12px 24px -12px rgb(0 0 0 / 0.4);
        }
        .bar { background: #202020; color: #fff; }
    }
</style>
</head>
<body>
<div class="stage"><div class="frame">
    <div class="bar">
        <span>Rollport</span>
        ${["M1 5h8", "M1.5 1.5h7v7h-7z", "M1.5 1.5l7 7M8.5 1.5l-7 7"]
            .map(
                (path) =>
                    `<b><svg viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="0.8"><path d="${path}"/></svg></b>`,
            )
            .join("")}
    </div>
    <!-- The demo makes the app take Windows' words: Ctrl, Explorer. -->
    <iframe src="/app/?os=windows"></iframe>
</div></div>
</body>
</html>`

const browser = await chromium.launch()

async function record(scheme: "light" | "dark", out: string) {
    const frames = join(tmpdir(), "rollport-clip")
    await rm(frames, { recursive: true, force: true })
    await mkdir(frames)

    const page = await browser.newPage({
        viewport: { width: WIDTH * SCALE, height: HEIGHT * SCALE },
        deviceScaleFactor: 1,
        colorScheme: scheme,
        reducedMotion: "no-preference",
    })
    // The fake's photo paths are relative to the app's page, under /app/.
    await page.route("http://app.local/**", (route) => {
        const path = new URL(route.request().url()).pathname
        if (path === "/")
            return route.fulfill({ contentType: "text/html", body: frame })
        if (path === "/app/")
            return route.fulfill({ contentType: "text/html", body: app })
        const photo = path.indexOf("/photos/")
        if (photo >= 0)
            return route.fulfill({ path: photos + path.slice(photo + 8) })
        return route.fulfill({ path: build + path })
    })

    const shots: { file: string; at: number }[] = []
    const cdp = await page.context().newCDPSession(page)
    // The page's own ground is see-through: the frames keep their alpha.
    await cdp.send("Emulation.setDefaultBackgroundColorOverride", {
        color: { r: 0, g: 0, b: 0, a: 0 },
    })
    let start = 0

    cdp.on("Page.screencastFrame", async ({ data, metadata, sessionId }) => {
        void cdp.send("Page.screencastFrameAck", { sessionId })
        const at = (metadata.timestamp ?? 0) * 1000 - start
        const last = shots.at(-1)
        if (at < FROM || at > UNTIL || (last && at - last.at < STEP)) return

        const file = join(
            frames,
            `${String(shots.length).padStart(4, "0")}.png`,
        )
        shots.push({ file, at })
        await writeFile(file, Buffer.from(data, "base64"))
    })

    await page.goto("http://app.local/")
    start = Date.now()
    await cdp.send("Page.startScreencast", { format: "png", everyNthFrame: 1 })
    await page.waitForTimeout(UNTIL + 500)
    await cdp.send("Page.stopScreencast")
    await page.close()

    // Each frame is shown until the next one; the last holds a second, so the
    // loop pauses on the report before it starts again. A keyframe every few
    // frames: with fewer, the slow tint behind the prints fills with blocks.
    const args = [
        "-loop",
        "0",
        "-kmin",
        "2",
        "-kmax",
        "4",
        "-lossy",
        "-q",
        "80",
        "-m",
        "6",
    ]
    shots.forEach((shot, i) => {
        const next = shots[i + 1]?.at ?? shot.at + 1000
        args.push(
            "-d",
            String(Math.max(Math.round(next - shot.at), STEP)),
            shot.file,
        )
    })
    execFileSync("img2webp", [...args, "-o", out], { stdio: "inherit" })
    await rm(frames, { recursive: true, force: true })

    const kb = Math.round((await stat(out)).size / 1024)
    console.log(`\n${shots.length} frames, ${kb} KB in ${out}`)
}

await record("light", docs + "rollport.webp")
await record("dark", docs + "rollport-dark.webp")
await browser.close()
