// The Rust side of Rollport, faked for the site: an iPhone with 312 new photos
// and videos, and an import that takes six seconds. scripts/demo.ts strips the
// types and runs this in the demo page before the app starts.
//
// The flows are typed with the app's own Flow, so a change there that this
// file does not follow fails the site's type check.
import type { Batch, Flow, Range, Run, Takes } from "../../src/lib/flow"

/** What the demo tells the page around it. */
export type DemoMessage =
    | { rollport: "height"; height: number }
    /** The photo in front, which the app tints its window with: a file in demo/photos/. */
    | { rollport: "photo"; name: string }

type Listener = (event: { event: string; id: number; payload: Flow }) => void

demo()

function demo() {
    // The site's dev switch can show another system's app (?os=windows): the
    // app reads the system from the user agent once, as it loads, after this.
    const os = new URLSearchParams(location.search).get("os")
    const agents: Record<string, string> = {
        mac: "Macintosh",
        windows: "Windows NT 10.0",
        linux: "X11; Linux x86_64",
    }
    const agent = os ? agents[os] : undefined
    if (agent) Object.defineProperty(navigator, "userAgent", { value: agent })

    const GB = 1_000_000_000
    const TOTAL = 312
    const BYTES = 4.1 * GB
    const RATE = 20_000_000
    /** How long the demo's import runs, and how long the camera roll takes to read. */
    const IMPORT_MS = 6000
    const COUNT_MS = 2400
    /** On ready, how long before the import starts by itself. */
    const IDLE_MS = 5000
    /** On done, how long before it reads the camera roll again. */
    const REST_MS = 4000

    const photo = (name: string) => `/DCIM/100APPLE/${name}.svg`
    const found = ["lake", "city", "sunset"]
    const landed = ["forest", "dunes", "flowers", "aurora", "beach", "forest"]

    /** A still for the site's pictures (scripts/demo.ts): one screen, held. */
    const still = new URLSearchParams(location.search).get("state")
    const auto =
        still === null &&
        !matchMedia("(prefers-reduced-motion: reduce)").matches

    const chosen = {
        state: "ready",
        udid: "00008130-001A",
        name: "iPhone",
        version: "26.0",
    } as const

    // The 312 new ones: 298 photos and 14 videos, no Live Photo clips, taken
    // over the days since the folder's last import, evenly, newest first.
    const PHOTOS = 298
    const VIDEOS = 14
    const DAY = 86_400_000
    const SPAN = 210

    /** The share of the new days a range reaches, as Rust counts by day. */
    function reach(range: Range): number {
        if (range.range === "everything") return 1
        if (range.range === "last") return Math.min(range.days / SPAN, 1)
        const today = Date.now()
        const from = Math.max(Date.parse(range.from), today - SPAN * DAY)
        const to = Math.min(Date.parse(range.to) + DAY, today)
        return Math.max(to - from, 0) / (SPAN * DAY)
    }

    /** Everything the folder takes in: what the camera roll holds. */
    const ALL: Batch = {
        files: 4522,
        bytes: 15.8e9,
        videos: 190,
        clips: 0,
        newest: ["sunset", "lake", "city"].map(photo),
    }

    /** What is new for a range and a choice of kinds (Takes in state.rs). */
    function wanted(range: Range, takes: Takes): Batch {
        const share = reach(range)
        const photos = takes === "videos" ? 0 : Math.round(PHOTOS * share)
        const videos = takes === "photos" ? 0 : Math.round(VIDEOS * share)
        const files = photos + videos
        // The newest prints are the latest days' photos; an older range, older ones.
        const recent =
            range.range !== "between" ||
            Date.parse(range.to) > Date.now() - 30 * DAY
        return {
            files,
            bytes: (BYTES * files) / TOTAL,
            videos,
            clips: 0,
            newest:
                files === 0
                    ? []
                    : (recent
                          ? ["sunset", "lake", "city"]
                          : ["forest", "beach", "dunes"]
                      ).map(photo),
        }
    }

    /** What is still new after `imported` of a batch landed. */
    function left(batch: Batch, imported: number): Batch {
        const share = batch.files > 0 ? 1 - imported / batch.files : 0
        return {
            ...batch,
            files: batch.files - imported,
            bytes: batch.bytes * share,
            videos: Math.round(batch.videos * share),
            newest: batch.files > imported ? batch.newest : [],
        }
    }

    /** How many of the new files a stopped import already copied. */
    let copied = 0

    /** What is new now, for the range and kinds the visitor chose. */
    function current(): Batch {
        const takes =
            flow.folder.folder === "known" ? flow.folder.takes : "everything"
        const batch = wanted(flow.range, takes)
        return left(batch, Math.min(copied, batch.files))
    }

    const ready: Flow = {
        source: { source: "listed", devices: [chosen], chosen },
        folder: {
            folder: "known",
            path: "~/Pictures/iPhone",
            frozen: true,
            lastImport: Date.parse("2026-03-04T18:00:00Z") / 1000,
            free: 500 * GB,
            network: false,
            // The default naming, as the help says a new folder starts with.
            layout: "{mtime:%Y-%m-%d_%H-%M-%S}_{name}",
            naming: "2026-09-02_15-04-11_IMG_0001.HEIC",
            takes: "everything",
        },
        roll: {
            roll: "done",
            all: ALL,
            new: wanted({ range: "everything" }, "everything"),
        },
        run: null,
        range: { range: "everything" },
    }

    // The stills that start somewhere other than ready.
    const trusting = {
        state: "waiting",
        udid: chosen.udid,
        need: { need: "trust" },
    } as const
    const starts: Record<string, Flow> = {
        // The first time: the iPhone asks to trust this computer, no folder yet.
        trust: {
            ...ready,
            source: { source: "listed", devices: [trusting], chosen: trusting },
            folder: { folder: "unset", suggested: "~/Pictures/iPhone" },
            roll: null,
        },
        // The iPhone talks to us; no folder yet.
        folder: {
            ...ready,
            folder: { folder: "unset", suggested: "~/Pictures/iPhone" },
            roll: null,
        },
    }

    let flow = (still !== null && starts[still]) || ready

    function counting(share: number): Flow {
        const latest = found[Math.floor(share * found.length)]
        return {
            ...flow,
            roll: {
                roll: "reading",
                seen: Math.round(4522 * share),
                latest: latest ? photo(latest) : null,
                failed: null,
            },
            run: null,
        }
    }

    function copying(
        share: number,
        batch: Batch,
    ): Extract<Run, { run: "copying" }> {
        const latest = photo(
            landed[Math.floor(share * landed.length)] ?? "forest",
        )
        return {
            run: "copying",
            done: Math.round(batch.files * share),
            total: batch.files,
            bytes: batch.bytes * share,
            totalBytes: batch.bytes,
            // Rust has no rate to quote at first.
            eta: share < 0.05 ? null : (batch.bytes * (1 - share)) / RATE,
            latest,
        }
    }

    function finished(imported: number, stopped: boolean, batch: Batch): Flow {
        return {
            ...flow,
            // As Rust: the folder now holds what landed, so less is new.
            roll: { roll: "done", all: ALL, new: left(batch, imported) },
            run: {
                run: "finished",
                imported,
                bytes: (batch.bytes * imported) / Math.max(batch.files, 1),
                seconds: Math.round(
                    (batch.bytes * imported) / Math.max(batch.files, 1) / RATE,
                ),
                end: stopped ? "stopped" : "done",
                trouble: null,
                newest: imported ? batch.newest : [],
                udid: chosen.udid,
            },
        }
    }

    const tell = (message: DemoMessage) =>
        window.parent.postMessage(message, "*")

    // --- The Tauri IPC the app's @tauri-apps/api reaches for.

    let next = 1
    const callbacks = new Map<number, Listener>()
    const listeners: number[] = []

    function publish(next: Flow) {
        flow = next
        for (const id of listeners)
            callbacks.get(id)?.({ event: "flow", id, payload: flow })
        idle()
    }

    // The page around the demo follows the app's tint: the photo in front,
    // blurred, in a wrapper with a mask (Sync.svelte). Read from the app's own
    // markup, so it changes when the app's tint does, not before. The newest
    // tint is the last one: an old one stays a moment while it fades out.
    let tinted: string | null = null
    new MutationObserver(() => {
        const images = document.querySelectorAll('[style*="mask-image"] img')
        const name =
            images[images.length - 1]?.getAttribute("src")?.split("/").pop() ??
            null
        if (name && name !== tinted) {
            tinted = name
            tell({ rollport: "photo", name })
        }
    }).observe(document.documentElement, { childList: true, subtree: true })

    // One timer at a time: whatever runs now is the only thing that will publish.
    let timer: ReturnType<typeof setTimeout> | undefined
    const stop = () => clearTimeout(timer)
    const later = (ms: number, then: () => void) => {
        stop()
        timer = setTimeout(then, ms)
    }

    /** Plays `ms` of something, publishing a flow each `step` ms. */
    function play(
        ms: number,
        step: number,
        at: (share: number) => Flow,
        then: () => void,
    ) {
        const start = performance.now()
        const tick = () => {
            const share = Math.min((performance.now() - start) / ms, 1)
            publish(at(share))
            if (share < 1) later(step, tick)
            else then()
        }
        tick()
    }

    /** What the running import copies, fixed as it starts, as in Rust. */
    let batch = current()

    function startImport() {
        batch = current()
        if (batch.files === 0) return
        play(
            IMPORT_MS,
            100,
            (share) => ({ ...flow, run: copying(share, batch) }),
            () => {
                // All of it is in the folder: the demo's next look finds 312 new again.
                copied = 0
                publish(finished(batch.files, false, batch))
                if (auto) later(REST_MS, rescan)
            },
        )
    }

    function rescan() {
        play(COUNT_MS, 80, counting, () => {
            flow = { ...ready, range: flow.range, folder: withSettings(ready) }
            publish({
                ...flow,
                roll: { roll: "done", all: ALL, new: current() },
            })
        })
    }

    /** Ready's folder, keeping what the visitor set in Options. */
    function withSettings(base: Flow) {
        if (base.folder.folder !== "known" || flow.folder.folder !== "known")
            return base.folder
        return {
            ...base.folder,
            takes: flow.folder.takes,
            layout: flow.folder.layout,
        }
    }

    /** On ready, start by itself unless the visitor gets there first. */
    function idle() {
        const resting = flow.run === null && flow.roll?.roll !== "reading"
        if (auto && resting) later(IDLE_MS, startImport)
    }

    /** The roll's new files again, for what the visitor chose; not mid-import. */
    function recount() {
        if (flow.run?.run === "copying" || flow.roll?.roll !== "done")
            return publish(flow)
        publish({ ...flow, run: null, roll: { ...flow.roll, new: current() } })
    }

    /** The suggested folder, then the camera roll read into it. */
    function chooseSuggested() {
        if (flow.folder.folder !== "unset") return
        publish({ ...flow, folder: withSettings(ready) })
        rescan()
    }

    let asked = false
    const commands: Record<string, (args: Record<string, unknown>) => unknown> =
        {
            flow: () => {
                // The still scripts/demo.ts drew while the app loaded: the
                // app draws this same screen in the next frame.
                // It goes once its photos are in (scripts/demo.ts), so the
                // app's photos are cached and its prints are never blank.
                const first = document.querySelector("[data-first]")
                const shown = (window as { __stillShown?: Promise<void> })
                    .__stillShown
                if (first)
                    void (shown ?? Promise.resolve()).then(() =>
                        requestAnimationFrame(() =>
                            requestAnimationFrame(() => first.remove()),
                        ),
                    )
                // Once the app has its first flow, a still goes to its screen.
                if (still !== null && !asked) {
                    asked = true
                    setTimeout(jump, 150)
                }
                return flow
            },
            "plugin:event|listen": (args) => {
                if (args["event"] === "flow")
                    listeners.push(args["handler"] as number)
                return next++
            },
            "plugin:window|set_size": (args) => {
                const value = args["value"] as { size: { height: number } }
                tell({
                    rollport: "height",
                    height: Math.round(value.size.height),
                })
            },
            start_import: () => startImport(),
            cancel_import: () => {
                stop()
                const run = flow.run
                const imported = run?.run === "copying" ? run.done : 0
                copied += imported
                publish(finished(imported, true, batch))
            },
            rescan: () => rescan(),
            // Rust counts again for the new range or kinds; so does this.
            set_range: (args) => {
                flow = { ...flow, range: args["range"] as Range }
                recount()
            },
            set_takes: (args) => {
                if (flow.folder.folder !== "known") return
                flow = {
                    ...flow,
                    folder: { ...flow.folder, takes: args["takes"] as Takes },
                }
                recount()
            },
            // The page cannot open a folder picker, Finder, or a second
            // iPhone: choosing a folder takes the suggested one, as its
            // button does, and the rest does nothing.
            use_suggested_destination: () => chooseSuggested(),
            choose_destination: () => chooseSuggested(),
            reveal_destination: () => null,
            // The help is this site: the demo sits in /demo/, the help beside it.
            show_help: ({ page }) =>
                open(
                    page === "troubleshooting"
                        ? "../help/troubleshooting"
                        : "../help",
                    "_top",
                ),
            choose_device: () => null,
            // The demo's folder has imported, so its naming is frozen, as in Rust.
            set_layout: () => {
                throw "Files here are already named this way."
            },
        }

    const internals = {
        metadata: {
            currentWindow: { label: "main" },
            currentWebview: { windowLabel: "main", label: "main" },
        },
        transformCallback(callback: Listener) {
            const id = next++
            callbacks.set(id, callback)
            return id
        },
        // The sample photos sit beside the demo page.
        convertFileSrc: (path: string) => `photos/${path.split("/").pop()}`,
        async invoke(command: string, args: Record<string, unknown> = {}) {
            return commands[command]?.(args) ?? null
        },
    }

    Object.assign(window, {
        __TAURI_INTERNALS__: internals,
        __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
    })

    idle()

    // A still jumps to its screen and stays. The app fills its prints from
    // the new photos on ready, so importing and done start from ready; the
    // photo "just landed" is one the app already shows, so nothing moves.
    const jumps: Record<string, () => void> = {
        importing: () =>
            publish({
                ...flow,
                run: { ...copying(0.63, current()), latest: photo("sunset") },
            }),
        done: () => publish(finished(TOTAL, false, current())),
    }
    function jump() {
        jumps[still ?? ""]?.()
        // scripts/demo.ts waits for this, past the app's own screen change.
        later(700, () => Object.assign(window, { __settled: true }))
    }
}
