// Every state the window can be in, photographed: the built app in WebKit
// (the engine the Mac app runs on) with a fake Rust side, in light, dark and
// Increase Contrast. Writes screens/index.html to look through them, and
// fails when a state throws or its content does not fit the window.
//
//     pnpm screens            all states
//     pnpm screens ready      only states whose name contains "ready"
import { execFileSync } from "node:child_process"
import { mkdir, readdir, rm, writeFile } from "node:fs/promises"
import { fileURLToPath } from "node:url"
import { webkit, type Page } from "playwright"
import type {
    Device,
    Flow,
    Folder,
    Report,
    Roll,
    Run,
} from "../src/lib/flow.ts"

const root = new URL("../", import.meta.url)
const build = fileURLToPath(new URL("build/", root))
const photos = fileURLToPath(new URL("site/demo/photos/", root))
const out = new URL("screens/", root)

// The facts, built up from a few pieces so each state says only what differs.

const iphone: Device = {
    state: "ready",
    udid: "00008110-000A",
    name: "Anna’s iPhone",
    version: "26.1",
}
const other: Device = { ...iphone, udid: "00008030-000B", name: "Work iPhone" }
const listed = (chosen: Device, ...more: Device[]): Flow["source"] => ({
    source: "listed",
    devices: [chosen, ...more],
    chosen,
})
const waiting = (need: Extract<Device, { state: "waiting" }>["need"]) =>
    listed({ state: "waiting", udid: iphone.udid, need })

const known: Folder = {
    folder: "known",
    path: "/Users/anna/Pictures/iPhone",
    frozen: true,
    lastImport: null,
    free: 500e9,
    network: false,
    layout: "{mtime:%Y}/{mtime:%m}/{name}",
    naming: "2026/09/IMG_4821.HEIC",
    takes: "everything",
}
const folder = (changes: Partial<Extract<Folder, { folder: "known" }>>) =>
    ({ ...known, ...changes }) as Folder

const NEWEST = [
    "/DCIM/100APPLE/IMG_4821.HEIC",
    "/DCIM/100APPLE/IMG_4820.HEIC",
    "/DCIM/100APPLE/IMG_4819.MOV",
]
const done = (files: number, fresh: number, newest = NEWEST): Roll => ({
    roll: "done",
    all: {
        files,
        bytes: files * 3.2e6,
        videos: Math.floor(files / 20),
        clips: Math.floor(files / 4),
        newest: [
            "/DCIM/100APPLE/IMG_4700.HEIC",
            "/DCIM/100APPLE/IMG_4699.HEIC",
            "/DCIM/100APPLE/IMG_4698.HEIC",
        ],
    },
    new: {
        files: fresh,
        bytes: fresh * 3.2e6,
        videos: Math.floor(fresh / 20),
        clips: Math.floor(fresh / 4),
        newest: fresh ? newest : [],
    },
})

const base: Flow = {
    source: listed(iphone),
    folder: known,
    roll: done(4210, 312),
    run: null,
    range: { range: "everything" },
}
const finished = (
    end: Report,
    roll: Roll | null = done(4210, 0),
): Partial<Flow> => ({
    run: { run: "finished", ...end } as Run,
    roll,
})
const report = (changes: Partial<Report> = {}): Report =>
    ({
        imported: 312,
        // What the files it copied weigh: a part run is part of 998 MB.
        bytes: Math.round(((changes.imported ?? 312) / 312) * 998e6),
        seconds: Math.round(((changes.imported ?? 312) / 312) * 94),
        trouble: null,
        end: "done",
        newest: (changes.imported ?? 312) ? NEWEST.slice(0, 2) : [],
        udid: iphone.udid,
        ...changes,
    }) as Report

type State = {
    name: string
    note: string
    flow: Flow
    /// What a person does after the screen appears, for states only reached by acting.
    act?: (page: Page) => Promise<unknown>
    /// What set_layout answers, when it refuses.
    refuse?: string
    /// The browser's user agent, for a screen another system words its own way.
    agent?: string
}
const state = (
    name: string,
    note: string,
    changes: Partial<Flow>,
    more: Partial<State> = {},
): State => ({
    name,
    note,
    flow: { ...base, ...changes },
    ...more,
})

const options = (page: Page) =>
    page.getByRole("button", { name: "Options" }).click()

const menu = (page: Page, name: string) =>
    page.getByRole("combobox", { name }).click()

async function choose(page: Page, name: string, item: string) {
    await menu(page, name)
    await page.getByRole("option", { name: item }).click()
}

const STATES: State[] = [
    // Setup: no folder yet, or one that will not open.
    state("setup-first-launch", "No folder, no phone", {
        source: { source: "missing" },
        folder: { folder: "unset", suggested: null },
        roll: null,
    }),
    state("setup-no-usbmuxd", "The system cannot list phones", {
        source: {
            source: "unavailable",
            reason: "connection refused (os error 61)",
        },
        folder: { folder: "unset", suggested: null },
        roll: null,
    }),
    state(
        "setup-no-usbmuxd-windows",
        "The system cannot list phones, on Windows",
        {
            source: {
                source: "unavailable",
                reason: "usbmuxd is unreachable: connection refused (os error 10061)",
            },
            folder: { folder: "unset", suggested: null },
            roll: null,
        },
        { agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64)" },
    ),
    state("setup-folder-suggested", "Phone ready, ~/Pictures/iPhone offered", {
        folder: { folder: "unset", suggested: "/Users/anna/Pictures/iPhone" },
        roll: null,
    }),
    state(
        "setup-folder-suggested-focused",
        "The main button with keyboard focus, as when a screen changes",
        {
            folder: {
                folder: "unset",
                suggested: "/Users/anna/Pictures/iPhone",
            },
            roll: null,
        },
        { act: (page) => page.locator(".btn-main").focus() },
    ),
    state("setup-folder-choose", "Phone ready, no folder to offer", {
        folder: { folder: "unset", suggested: null },
        roll: null,
    }),
    state("setup-folder-opening", "The chosen folder is being read", {
        folder: { folder: "opening" },
        roll: null,
    }),
    state("setup-folder-broken", "The chosen folder cannot be used", {
        folder: {
            folder: "broken",
            path: known.path,
            reason: "Permission denied (os error 13)",
        },
        roll: null,
    }),
    state(
        "setup-broken-no-phone",
        "A broken folder and no phone: the phone is asked for first",
        {
            source: { source: "missing" },
            folder: {
                folder: "broken",
                path: known.path,
                reason: "No such file or directory (os error 2)",
            },
            roll: null,
        },
    ),

    // Prompts: a folder, and a phone that will not talk yet.
    state("prompt-no-usbmuxd", "", {
        source: {
            source: "unavailable",
            reason: "connection refused (os error 61)",
        },
        roll: null,
    }),
    state(
        "prompt-no-usbmuxd-linux",
        "",
        {
            source: {
                source: "unavailable",
                reason: "connection refused (os error 111)",
            },
            roll: null,
        },
        { agent: "Mozilla/5.0 (X11; Linux x86_64)" },
    ),
    state("prompt-connect", "", { source: { source: "missing" }, roll: null }),
    state("prompt-trust", "", {
        source: waiting({ need: "trust" }),
        roll: null,
    }),
    state("prompt-unlock", "", {
        source: waiting({ need: "unlock" }),
        roll: null,
    }),
    state("prompt-trust-refused", "", {
        source: waiting({ need: "replug" }),
        roll: null,
    }),
    state("prompt-connect-failed", "", {
        source: waiting({
            need: "failed",
            reason: "lockdown refused the connection: the iPhone stopped responding",
        }),
        roll: null,
    }),
    state("prompt-roll-locked", "A trusted iPhone, locked", {
        roll: {
            roll: "reading",
            seen: 0,
            latest: null,
            failed: "the device is locked",
        },
    }),

    // Counting and up to date.
    state("counting-start", "Before the first file is found", { roll: null }),
    state(
        "counting",
        "",
        {
            roll: {
                roll: "reading",
                seen: 1284,
                latest: NEWEST[0],
                failed: null,
            },
        },
        // The photo just found is dealt within a second (DEAL in Sync).
        { act: (page) => page.waitForTimeout(1500) },
    ),
    state("current-first", "Up to date, never imported here", {
        roll: done(4210, 0),
        folder: folder({ lastImport: null }),
    }),
    state("current", "Up to date, last import today", {
        roll: done(4210, 0),
        folder: folder({ lastImport: Math.floor(Date.now() / 1000) - 3600 }),
    }),

    // Ready.
    state("ready", "", {}),
    state("ready-one", "One photo", {
        roll: done(4210, 1, NEWEST.slice(0, 1)),
    }),
    state(
        "ready-no-thumbnails",
        "Photos without thumbnails are drawn as paper",
        {
            roll: done(4210, 3, [
                "/DCIM/100APPLE/IMG_0001.DNG",
                "/DCIM/100APPLE/IMG_0002.DNG",
                "/DCIM/100APPLE/IMG_0003.DNG",
            ]),
        },
    ),
    state("ready-full-disk", "The warning that comes first", {
        folder: folder({ free: 120e6 }),
    }),
    state("ready-network", "", { folder: folder({ network: true }) }),
    state("ready-full-network", "A full disk is said before the network", {
        folder: folder({ free: 120e6, network: true }),
    }),
    state("ready-two-phones", "The phone pill is a menu", {
        source: listed(iphone, other),
    }),
    state(
        "ready-range-and-takes",
        "The footer says the range and the media types",
        {
            range: { range: "between", from: "2026-06-01", to: "2026-08-31" },
            folder: folder({ takes: "photos" }),
            // What Rust counts with them: photos only, from those days.
            roll: {
                ...done(4210, 0),
                new: {
                    files: 86,
                    bytes: 86 * 3.2e6,
                    videos: 0,
                    clips: 0,
                    newest: NEWEST.slice(0, 2),
                },
            },
        },
    ),
    state("ready-long-names", "A long phone name and folder path", {
        source: listed({
            ...iphone,
            name: "Anna Katharina’s iPhone 17 Pro Max",
        }),
        folder: folder({
            path: "/Volumes/Photo Archive 2026/Family/Anna/iPhone imports/September",
        }),
    }),

    state("ready-drive", "A folder on a drive with a short name", {
        folder: folder({
            path: "/Volumes/PHOTOS/Family/iPhone imports/September",
        }),
    }),

    // Importing.
    state("importing-start", "The rate is not worth quoting yet", {
        run: {
            run: "copying",
            done: 0,
            total: 312,
            bytes: 0,
            totalBytes: 998e6,
            eta: null,
            latest: null,
        },
    }),
    state("importing", "", {
        run: {
            run: "copying",
            done: 141,
            total: 312,
            bytes: 451e6,
            totalBytes: 998e6,
            eta: 118,
            latest: NEWEST[1],
        },
    }),

    // The report, and the button it offers.
    state(
        "report-imported-rescan",
        "Phone still here, nothing left",
        finished(report()),
    ),
    state("report-imported-unplugged", "Phone gone after a full run", {
        ...finished(report()),
        source: { source: "missing" },
    }),
    state(
        "report-roll-reading",
        "Phone here, what is left is still being counted",
        finished(report(), {
            roll: "reading",
            seen: 200,
            latest: null,
            failed: null,
        }),
    ),
    state(
        "report-nothing",
        "A run that found nothing to take",
        finished(report({ imported: 0, bytes: 0, seconds: 1 })),
    ),
    state(
        "report-stopped-continue",
        "Stopped, with files left",
        finished(report({ end: "stopped", imported: 140 }), done(4210, 172)),
    ),
    state("report-full", "Stopped before the disk was too full", {
        ...finished(report({ end: "full", imported: 140 }), done(4210, 172)),
        folder: folder({ free: 120e6 }),
    }),
    state("report-full-none", "Not even the first file fit", {
        ...finished(
            report({ end: "full", imported: 0, bytes: 0 }),
            done(4210, 312),
        ),
        folder: folder({ free: 120e6 }),
    }),
    state(
        "report-stopped-none",
        "Stopped before the first file",
        finished(
            report({ end: "stopped", imported: 0, bytes: 0 }),
            done(4210, 312),
        ),
    ),
    state("report-stopped-unplugged", "Stopped, then the phone was unplugged", {
        ...finished(report({ end: "stopped", imported: 140 }), null),
        source: { source: "missing" },
    }),
    state("report-unplugged", "The cable came out during the copy", {
        ...finished(report({ end: "unplugged", imported: 140 }), null),
        source: { source: "missing" },
    }),
    state(
        "report-unplugged-counting",
        "Connected again, what is left is still being counted",
        finished(report({ end: "unplugged", imported: 140 }), {
            roll: "reading",
            seen: 200,
            latest: null,
            failed: null,
        }),
    ),
    state("report-unplugged-locked", "Connected again, but locked", {
        ...finished(report({ end: "unplugged", imported: 140 }), null),
        source: waiting({ need: "unlock" }),
    }),
    state(
        "report-unplugged-unreadable",
        "Connected again, but its files cannot be read yet",
        finished(report({ end: "unplugged", imported: 140 }), {
            roll: "reading",
            seen: 0,
            latest: null,
            failed: "the iPhone is locked",
        }),
    ),
    state("report-unplugged-other-phone", "Another iPhone is there", {
        ...finished(report({ end: "unplugged", imported: 140 }), null),
        source: { source: "listed", devices: [other], chosen: null },
    }),
    state(
        "report-unplugged-continue",
        "Connected again, with files left",
        finished(report({ end: "unplugged", imported: 140 }), done(4210, 172)),
    ),
    state(
        "report-trouble-one",
        "One file could not be copied",
        finished(
            report({
                trouble: {
                    failed: 1,
                    name: "IMG_4790.HEIC",
                    reason: "the file vanished from the iPhone",
                },
            }),
            done(4210, 1),
        ),
    ),
    state(
        "report-stopped-trouble",
        "Stopped, and a file before it would not copy",
        finished(
            report({
                end: "stopped",
                imported: 140,
                trouble: {
                    failed: 1,
                    name: "IMG_4790.HEIC",
                    reason: "the file vanished from the iPhone",
                },
            }),
            done(4210, 172),
        ),
    ),
    state(
        "report-trouble-none",
        "Not one file could be copied",
        finished(
            report({
                imported: 0,
                bytes: 0,
                trouble: {
                    failed: 312,
                    name: "IMG_4509.HEIC",
                    reason: "the iPhone stopped responding",
                },
            }),
            done(4210, 312),
        ),
    ),
    state("report-full-unplugged", "Stopped for space, then unplugged", {
        ...finished(report({ end: "full", imported: 140 }), null),
        source: { source: "missing" },
        folder: folder({ free: 120e6 }),
    }),
    state(
        "report-folder-broken",
        "The folder broke at the end of a run: the folder step, not the report",
        {
            ...finished(report(), done(4210, 0)),
            folder: {
                folder: "broken",
                path: known.path,
                reason: "cannot find the folder",
            },
        },
    ),
    state(
        "report-trouble-many-unplugged",
        "Many files lost as the cable came out",
        {
            ...finished(
                report({
                    imported: 88,
                    trouble: {
                        failed: 224,
                        name: "IMG_4600.HEIC",
                        reason: "the connection to the iPhone was lost",
                    },
                }),
                null,
            ),
            source: { source: "missing" },
        },
    ),
    state(
        "report-failed",
        "The folder ended the run",
        finished(
            report({
                end: "failed",
                imported: 97,
                reason: "No space left on device (os error 28)",
            }),
            done(4210, 215),
        ),
    ),
    state(
        "report-failed-locked",
        "Another import holds the folder",
        finished(
            report({
                end: "failed",
                imported: 0,
                reason: "Another import is writing into this folder. Import again once it is done.",
            }),
            done(4210, 312),
        ),
    ),
    state(
        "report-failed-long-reason",
        "A long reason is clamped to two lines",
        finished(
            report({
                end: "failed",
                imported: 0,
                bytes: 0,
                reason: "Input/output error (os error 5) while writing /Volumes/Photo Archive 2026/Family/Anna/iPhone imports/September/2026/09/IMG_4821.HEIC.part",
            }),
            done(4210, 312),
        ),
    ),

    // Reached by acting.
    state("options", "The Options popover", {}, { act: options }),
    state(
        "options-menu-kinds",
        "The Media types menu open, with its second lines",
        {},
        {
            act: async (page) => {
                await options(page)
                await menu(page, "Media types, saved with this folder")
            },
        },
    ),
    state(
        "options-menu",
        "A menu open over the Options",
        { folder: folder({ frozen: false }) },
        {
            act: async (page) => {
                await options(page)
                await menu(page, "File names, saved with this folder")
            },
        },
    ),
    state(
        "ready-two-phones-menu",
        "The phone menu open",
        { source: listed(iphone, other) },
        { act: (page) => menu(page, "iPhone to import from") },
    ),
    state(
        "prompt-refused-other-phone-menu",
        "Trust refused on one phone, another ready: the menu goes back",
        {
            source: listed(
                {
                    state: "waiting",
                    udid: "00008101-000C",
                    need: { need: "replug" },
                },
                iphone,
            ),
            roll: null,
        },
        { act: (page) => menu(page, "iPhone to import from") },
    ),
    state(
        "report-unplugged-other-phone-menu",
        "The report waits; the other iPhone can be chosen",
        {
            ...finished(report({ end: "unplugged", imported: 140 }), null),
            source: { source: "listed", devices: [other], chosen: null },
        },
        { act: (page) => menu(page, "iPhone to import from") },
    ),
    state(
        "options-between",
        "Two dates",
        { range: { range: "between", from: "2026-06-01", to: "2026-08-31" } },
        { act: options },
    ),
    state(
        "options-own-pattern",
        "A folder that has not named files yet: a pattern of your own",
        {
            folder: folder({
                frozen: false,
                layout: "{name}",
                naming: "IMG_4821.HEIC",
            }),
        },
        {
            act: async (page) => {
                await options(page)
                await choose(
                    page,
                    "File names, saved with this folder",
                    "A pattern of your own",
                )
            },
        },
    ),
    state(
        "options-refused-pattern",
        "Rust refuses the pattern and says why",
        {
            folder: folder({
                frozen: false,
                layout: "{name}",
                naming: "IMG_4821.HEIC",
            }),
        },
        {
            // What check_layout says of {mtime:%Y}.
            refuse: "Add {name}. Without it, every file gets the same name.",
            act: async (page) => {
                await options(page)
                await choose(
                    page,
                    "File names, saved with this folder",
                    "A pattern of your own",
                )
                await page.getByLabel("Naming pattern").fill("{mtime:%Y}")
                await page.getByLabel("Naming pattern").press("Enter")
            },
        },
    ),
    state(
        "drop-folder",
        "A folder dragged over the window",
        {},
        {
            act: (page) =>
                page.evaluate(() =>
                    (window as any).__emit("tauri://drag-enter", {
                        paths: ["/Users/anna/Desktop/Photos"],
                        position: { x: 200, y: 300 },
                    }),
                ),
        },
    ),
]

const THEMES = [
    { name: "light", colorScheme: "light", contrast: "no-preference" },
    { name: "dark", colorScheme: "dark", contrast: "no-preference" },
    { name: "contrast", colorScheme: "light", contrast: "more" },
] as const

/// Runs in the page before the app: the Tauri globals the app calls, answering
/// from `flow`. Thumbnails come from the site demo's sample photos.
function fake({ flow, refuse }: { flow: Flow; refuse: string | null }) {
    const callbacks = new Map<number, (message: unknown) => void>()
    const listeners = new Map<string, number[]>()
    let next = 1
    const w = window as any

    w.__emit = (event: string, payload: unknown) =>
        (listeners.get(event) ?? []).forEach((id) =>
            callbacks.get(id)?.({ event, id, payload }),
        )

    w.__TAURI_INTERNALS__ = {
        metadata: {
            currentWindow: { label: "main" },
            currentWebview: { windowLabel: "main", label: "main" },
        },
        transformCallback(callback: (message: unknown) => void) {
            callbacks.set(next, callback)
            return next++
        },
        convertFileSrc(path: string) {
            // DNGs have no thumbnail on the phone, as some do not.
            if (path.endsWith(".DNG")) return "http://photos.local/missing"
            // The phone before the path: the same photos whichever phone.
            path = path.slice(path.indexOf("/"))
            const names = [
                "sunset",
                "beach",
                "forest",
                "city",
                "lake",
                "dunes",
                "flowers",
                "aurora",
            ]
            // Weighted by place, so neighbouring names get different photos.
            const n = [...path].reduce(
                (hash, c) => (hash * 31 + c.charCodeAt(0)) >>> 0,
                0,
            )
            return `http://photos.local/${names[n % names.length]}.svg`
        },
        async invoke(command: string, args: any) {
            if (command === "plugin:event|listen") {
                listeners.set(args.event, [
                    ...(listeners.get(args.event) ?? []),
                    args.handler,
                ])
                return args.handler
            }
            if (command === "flow") return flow
            if (command === "plugin:app|version") return "0.1.0"
            if (command === "set_layout" && refuse) throw refuse
            return null
        },
    }
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} }
}

/// Anything drawn within 16 px of the footer's top edge, or clipped by the
/// screen it is in: a screen that only just fits breaks on the next longer line.
async function overflow(page: Page) {
    return page.evaluate(() => {
        const stage = document.querySelector<HTMLElement>("[data-stage]")
        const floor =
            document
                .querySelector("footer, main > p.border-t")
                ?.getBoundingClientRect().top ?? innerHeight
        const low = [...(stage?.querySelectorAll("*") ?? [])].filter(
            (e) =>
                !e.closest("[aria-hidden='true'], .absolute") &&
                // What draws something; the spacers reach the footer empty.
                (e.matches("button, input, select, textarea, img, svg") ||
                    [...e.childNodes].some(
                        (n) => n.nodeType === 3 && n.textContent?.trim(),
                    )) &&
                e.getClientRects().length &&
                e.getBoundingClientRect().bottom > floor - 16 + 0.5,
        )
        return low.map(
            (e) =>
                (e.textContent ?? e.tagName).trim().slice(0, 40) || e.tagName,
        )
    })
}

const only = process.argv[2]
const states = STATES.filter((s) => !only || s.name.includes(only))

execFileSync("pnpm", ["build"], { cwd: fileURLToPath(root), stdio: "inherit" })
await rm(out, { recursive: true, force: true })
await mkdir(out, { recursive: true })

const browser = await webkit.launch()
const results: { state: State; problems: string[] }[] = []
/// Where each screen's art starts: it should not move between screens.
const arts: [string, number | null][] = []

for (const s of states) {
    const problems: string[] = []

    for (const theme of THEMES) {
        const context = await browser.newContext({
            viewport: { width: 400, height: 560 },
            deviceScaleFactor: 2,
            ...(s.agent && { userAgent: s.agent }),
            colorScheme: theme.colorScheme,
            contrast: theme.contrast,
            // Still pictures of a moving window: every state at rest.
            reducedMotion: "reduce",
        })
        await context.addInitScript(fake, {
            flow: s.flow,
            refuse: s.refuse ?? null,
        })
        await context.route("http://photos.local/**", (route) => {
            const name = new URL(route.request().url()).pathname.slice(1)
            return name.endsWith(".svg")
                ? route.fulfill({ path: photos + name })
                : route.fulfill({ status: 404 })
        })
        await context.route("http://app.local/**", (route) => {
            const path = new URL(route.request().url()).pathname
            return route.fulfill({
                path: build + (path === "/" ? "index.html" : path),
            })
        })

        const page = await context.newPage()
        page.on("pageerror", (error) =>
            problems.push(`${theme.name}: ${error.message}`),
        )
        await page.goto("http://app.local/")
        await page.waitForLoadState("networkidle")
        if (s.act) await s.act(page)
        await page.waitForTimeout(400)

        if (theme === THEMES[0])
            arts.push([
                s.name,
                await page.evaluate(
                    () =>
                        document
                            .querySelector("[data-art]")
                            ?.getBoundingClientRect().top ?? null,
                ),
            ])
        for (const low of await overflow(page))
            problems.push(`${theme.name}: too near the footer: ${low}`)
        await page.screenshot({
            path: fileURLToPath(new URL(`${s.name}.${theme.name}.png`, out)),
        })
        await context.close()
    }

    results.push({ state: s, problems: [...new Set(problems)] })
    console.log(
        `${problems.length ? "✗" : "✓"} ${s.name}${problems.length ? `\n    ${problems.join("\n    ")}` : ""}`,
    )
}
await browser.close()

const escape = (text: string) =>
    text.replace(/[&<>"]/g, (c) => `&#${c.charCodeAt(0)};`)
const rows = results
    .map(
        ({ state, problems }) => `
<section${problems.length ? ' class="bad"' : ""} id="${state.name}">
  <h2><a href="#${state.name}">${state.name}</a></h2>
  ${state.note ? `<p>${escape(state.note)}</p>` : ""}
  ${problems.map((p) => `<p class="problem">${escape(p)}</p>`).join("")}
  <div>${THEMES.map((t) => `<figure><img src="${state.name}.${t.name}.png" width="400" height="560" alt="${state.name}, ${t.name}"><figcaption>${t.name}</figcaption></figure>`).join("")}</div>
</section>`,
    )
    .join("")
await writeFile(
    new URL("index.html", out),
    `<!doctype html>
<meta charset="utf-8">
<title>Screens</title>
<style>
  body { font: 14px/1.4 system-ui, sans-serif; margin: 24px; background: #f1f0ef; color: #21201c }
  section { margin: 0 0 40px }
  h2 { font-size: 16px; margin: 0 0 4px } h2 a { color: inherit }
  p { margin: 0 0 8px; color: #63635e } .problem { color: #d13415; font-weight: 600 }
  div { display: flex; gap: 16px; flex-wrap: wrap } figure { margin: 0 }
  img { display: block; border-radius: 10px; box-shadow: 0 1px 3px #0003 }
  figcaption { font-size: 12px; color: #63635e; margin-top: 4px }
</style>
<h1>${results.length} states, ${THEMES.length} looks each</h1>
${rows}
`,
)

const tops = Map.groupBy(arts, ([, top]) => top)
console.log("\nArt top, and the screens that put it there:")
for (const [top, names] of [...tops].sort(([a], [b]) => (a ?? 0) - (b ?? 0)))
    console.log(`  ${top ?? "none"}: ${names.map(([name]) => name).join(", ")}`)

const bad = results.filter((r) => r.problems.length).length
const shots = (await readdir(out)).filter((f) => f.endsWith(".png")).length
console.log(
    `\n${shots} screenshots in ${fileURLToPath(new URL("index.html", out))}${bad ? `; ${bad} states with problems` : ""}`,
)
process.exitCode = bad ? 1 : 0
