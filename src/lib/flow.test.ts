// What the window shows, from the facts: every screen and the one button after
// a run. Run with `pnpm test`. Most of these need a cable pulled at the right
// moment to see by hand.
import assert from "node:assert/strict"
import { describe, test } from "node:test"
import {
    QUIET,
    screen,
    duration,
    elapsed,
    since,
    size,
    type Batch,
    type Device,
    type Flow,
    type Report,
    type Roll,
    type Screen,
    type Source,
} from "./flow.ts"

const phone: Device = {
    state: "ready",
    udid: "00008110-000A",
    name: "Anna’s iPhone",
    version: "26.1",
}
const waiting = (need: "trust" | "unlock"): Source => ({
    source: "listed",
    devices: [{ state: "waiting", udid: phone.udid, need: { need } }],
    chosen: { state: "waiting", udid: phone.udid, need: { need } },
})
const ready: Source = { source: "listed", devices: [phone], chosen: phone }
const missing: Source = { source: "missing" }

const batch = (files: number, bytes = files * 1e6): Batch => ({
    files,
    bytes,
    videos: 0,
    clips: 0,
    newest: [],
})
const read = (fresh: number, bytes?: number): Roll => ({
    roll: "done",
    all: batch(fresh + 100),
    new: batch(fresh, bytes),
})
const reading = (failed: string | null = null): Roll => ({
    roll: "reading",
    seen: 0,
    latest: null,
    failed,
})

const known: Extract<Flow["folder"], { folder: "known" }> = {
    folder: "known",
    path: "~/Pictures/iPhone",
    frozen: false,
    lastImport: null,
    free: 500e9,
    network: false,
    layout: "{name}",
    naming: "IMG_0001.HEIC",
    takes: "everything",
}

const flow = (changes: Partial<Flow> = {}): Flow => ({
    ...QUIET,
    source: ready,
    folder: known,
    roll: read(0),
    ...changes,
})

const finished = (changes: Partial<Report> = {}): Flow["run"] =>
    ({
        run: "finished",
        imported: 140,
        bytes: 998e6,
        seconds: 94,
        trouble: null,
        end: "done",
        newest: [],
        udid: phone.udid,
        ...changes,
    }) as Flow["run"]

/// The report a flow shows, or a failure saying what it showed instead.
function done(of: Flow): Extract<Screen, { screen: "done" }> {
    const shown = screen(of)
    assert.equal(shown.screen, "done")
    return shown as Extract<Screen, { screen: "done" }>
}

describe("the report's button and the line under it", () => {
    const cases: [string, Flow, { next: string | null; after: string }][] = [
        [
            "a stopped run waits for the count, with Continue shown",
            flow({ run: finished({ end: "stopped" }), roll: null }),
            { next: "counting", after: "Continue imports what is left." },
        ],
        [
            "a run that did all of it shows no button while it counts",
            flow({ run: finished(), roll: null }),
            { next: null, after: "Looking through the camera roll…" },
        ],
        [
            "files left: Continue",
            flow({ run: finished(), roll: read(3) }),
            { next: "continue", after: "Continue imports what is left." },
        ],
        [
            "a run that copied nothing starts again",
            flow({
                run: finished({ end: "full", imported: 0, bytes: 0 }),
                roll: read(3),
            }),
            { next: "continue", after: "Import starts again." },
        ],
        [
            "nothing left: Check again",
            flow({ run: finished(), roll: read(0) }),
            { next: "rescan", after: "You can unplug the iPhone." },
        ],
        [
            "the cable came out and the phone is gone",
            flow({ run: finished({ end: "unplugged" }), source: missing }),
            { next: null, after: "Connect the iPhone again to continue." },
        ],
        [
            "the cable came out and another phone is there",
            flow({
                run: finished({ end: "unplugged" }),
                source: { source: "listed", devices: [phone], chosen: null },
            }),
            { next: null, after: "Connect the iPhone again to continue." },
        ],
        [
            "back, but locked",
            flow({
                run: finished({ end: "unplugged" }),
                source: waiting("unlock"),
            }),
            { next: null, after: "Unlock the iPhone to continue." },
        ],
        [
            "back, but asking to be trusted",
            flow({
                run: finished({ end: "stopped" }),
                source: waiting("trust"),
            }),
            { next: null, after: "Tap Trust on the iPhone to continue." },
        ],
        [
            "back, but its files cannot be read yet",
            flow({
                run: finished({ end: "unplugged" }),
                roll: reading("locked"),
            }),
            { next: "counting", after: "Unlock the iPhone to continue." },
        ],
        [
            "a run that did all of it, its phone gone",
            flow({ run: finished(), source: missing }),
            { next: null, after: "Connect the iPhone to look for new photos." },
        ],
        [
            "a stopped run, its phone gone",
            flow({ run: finished({ end: "stopped" }), source: missing }),
            { next: null, after: "Connect the iPhone again to continue." },
        ],
        [
            "a run that did all of it, back but its files cannot be read yet",
            flow({ run: finished(), roll: reading("locked") }),
            { next: null, after: "Unlock the iPhone to look for new photos." },
        ],
        [
            "a run that did all of it needs nothing from a locked phone",
            flow({ run: finished(), source: waiting("unlock") }),
            { next: null, after: "You can unplug the iPhone." },
        ],
    ]

    for (const [name, of, expected] of cases)
        test(name, () => {
            const { next, after } = done(of)
            assert.deepEqual({ next, after }, expected)
        })
})

describe("the report's title, badge and words", () => {
    test("a stop that also lost files carries the trouble badge", () => {
        const shown = done(
            flow({
                run: finished({
                    end: "stopped",
                    trouble: {
                        failed: 2,
                        name: "IMG_0001.HEIC",
                        reason: "gone",
                    },
                }),
            }),
        )
        assert.equal(shown.title, "Stopped after 140")
        assert.equal(shown.badge, "trouble")
    })

    test("a full disk before the first file", () => {
        const shown = done(
            flow({
                run: finished({ end: "full", imported: 0, bytes: 0 }),
                roll: read(3, 5e9),
                folder: { ...known, free: 1e9 },
            }),
        )
        assert.equal(shown.title, "Not enough space")
        assert.equal(shown.badge, "full")
        assert.match(shown.detail, /^Free up 4(\.0)? GB to import\.$/)
    })

    test("a full disk after some: what fits is in, the rest needs room", () => {
        const shown = done(
            flow({
                run: finished({ end: "full" }),
                roll: read(3, 5e9),
                folder: { ...known, free: 1e9 },
            }),
        )
        assert.equal(shown.title, "Imported 140")
        assert.equal(shown.badge, "full")
        assert.match(
            shown.detail,
            /Free up 4(\.0)?\u00A0GB to import the rest\.$/,
        )
    })

    test("one file that would not copy is named", () => {
        const shown = done(
            flow({
                run: finished({
                    trouble: {
                        failed: 1,
                        name: "IMG_0001.HEIC",
                        reason: "gone",
                    },
                }),
            }),
        )
        assert.match(
            shown.detail,
            /IMG_0001\.HEIC could not be copied and is still on the iPhone\.$/,
        )
        assert.equal(shown.reason, "gone")
    })

    test("a file lost without a name is not called one", () => {
        const shown = done(
            flow({
                run: finished({
                    trouble: {
                        failed: 2,
                        name: "",
                        reason: "the copy stopped unexpectedly",
                    },
                }),
            }),
        )
        assert.doesNotMatch(shown.detail, /The first was/)
    })

    test("a failed run passes on what the machine said", () => {
        const shown = done(
            flow({ run: finished({ end: "failed", reason: "disk gone" }) }),
        )
        assert.equal(shown.title, "Import failed")
        assert.equal(shown.reason, "disk gone")
    })

    test("another import in the folder is a wait, not a failure", () => {
        const shown = done(
            flow({
                run: finished({
                    end: "failed",
                    imported: 0,
                    reason: "Another import is writing into this folder. Import again once it is done.",
                }),
            }),
        )
        assert.equal(shown.title, "Another import is running")
        assert.equal(shown.badge, "stopped")
        assert.equal(shown.reason, "")
    })

    test("a camera roll that cannot be read is not said to be counted", () => {
        const shown = done(
            flow({
                run: finished({ end: "unplugged" }),
                roll: reading("locked"),
            }),
        )
        assert.doesNotMatch(shown.detail, /Counting/)
    })

    test("a broken folder is named in its advice", () => {
        const shown = screen(
            flow({
                folder: {
                    folder: "broken",
                    path: "~/Pictures/iPhone",
                    reason: "gone",
                },
            }),
        )
        assert.ok(shown.screen === "setup")
        assert.match(shown.said.detail, /~\/Pictures\/iPhone is there/)
    })

    test("a pulled cable says so", () => {
        const shown = done(flow({ run: finished({ end: "unplugged" }) }))
        assert.equal(shown.title, "Disconnected after 140")
        assert.equal(shown.badge, "unplugged")
    })
})

describe("which screen wins", () => {
    test("a copy outranks the cable", () => {
        const shown = screen(
            flow({
                source: waiting("unlock"),
                run: {
                    run: "copying",
                    done: 1,
                    total: 2,
                    bytes: 1,
                    totalBytes: 2,
                    eta: null,
                    latest: null,
                },
            }),
        )
        assert.equal(shown.screen, "importing")
    })

    test("a report outranks a missing phone", () => {
        assert.equal(
            screen(flow({ source: missing, run: finished() })).screen,
            "done",
        )
    })

    test("a phone that waits is asked for before its camera roll", () => {
        const shown = screen(flow({ source: waiting("trust"), roll: null }))
        assert.equal(shown.screen, "prompt")
    })

    test("a camera roll that cannot be read asks for the phone unlocked", () => {
        const shown = screen(flow({ roll: reading("the iPhone is locked") }))
        assert.ok(shown.screen === "prompt")
        assert.equal(shown.said.title, "Unlock the iPhone")
        assert.equal(shown.said.art, "lock")
    })

    test("a camera roll that will not open for another reason is a failure", () => {
        const shown = screen(
            flow({ roll: reading("AFC refused the connection") }),
        )
        assert.ok(shown.screen === "prompt")
        assert.equal(shown.said.title, "Cannot read the camera roll")
        assert.equal(shown.said.art, "trouble")
    })

    test("a full disk is said before a network drive", () => {
        const shown = screen(
            flow({
                roll: read(3, 5e9),
                folder: {
                    ...known,
                    free: 1e9,
                    network: true,
                },
            }),
        )
        assert.ok(shown.screen === "ready")
        assert.equal(shown.warning?.tone, "short")
    })

    test("nothing new and never imported: no last import is named", () => {
        const shown = screen(flow({ roll: read(0) }))
        assert.ok(shown.screen === "current")
        assert.doesNotMatch(shown.detail, /Last import/)
    })

    test("no folder yet: the phone's need comes first", () => {
        const shown = screen(
            flow({
                source: waiting("trust"),
                folder: { folder: "unset", suggested: null },
            }),
        )
        assert.ok(shown.screen === "setup")
        assert.equal(shown.said.title, "Tap Trust on the iPhone")
    })
})

describe("sizes and times, as a person says them", () => {
    test("a size is said as it rounds", () => {
        for (const [bytes, said] of [
            [0, "0 B"],
            [999, "999 B"],
            [1_000, "1.0 KB"],
            [9_940_000, "9.9 MB"],
            [9_960_000, "10 MB"],
            [999_499, "999 KB"],
            [999_950, "1.0 MB"],
            [999_600_000, "1.0 GB"],
        ] as const)
            assert.equal(size(bytes), said.replace(" ", "\u00A0"))
    })

    test("a span that has happened", () => {
        for (const [seconds, said] of [
            [0, "1 second"],
            [59, "59 seconds"],
            [60, "1 minute"],
            [89, "1 minute"],
            [3599, "1 hour"],
            [5400, "1 hour 30 minutes"],
            [7200, "2 hours"],
        ] as const)
            assert.equal(elapsed(seconds), said)
    })

    test("time left is not a number under a minute", () => {
        assert.equal(duration(44), "Less than a minute left")
        assert.equal(duration(45), "About 1 minute left")
        assert.equal(duration(150), "About 3 minutes left")
    })

    test("a day is today, yesterday, or a date with its year only if not this one", () => {
        const noon = new Date()
        noon.setHours(12, 0, 0, 0)
        const at = (date: Date) => date.getTime() / 1000
        const daysBack = (days: number) => {
            const day = new Date(noon)
            day.setDate(day.getDate() - days)
            return day
        }

        assert.equal(since(at(noon)), "today")
        assert.equal(since(at(daysBack(1))), "yesterday")

        const lastYear = new Date(noon.getFullYear() - 1, 2, 4, 12)
        assert.match(
            since(at(lastYear)),
            new RegExp(`${lastYear.getFullYear()}`),
        )

        // Three days back is this year unless today is 1 to 3 January.
        const earlier = daysBack(3)
        if (earlier.getFullYear() === noon.getFullYear())
            assert.doesNotMatch(since(at(earlier)), /\d{4}/)
    })
})
