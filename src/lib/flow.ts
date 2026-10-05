import { invoke } from "@tauri-apps/api/core"

// The shape Rust publishes, mirrored for the window to render. Names match
// their Rust counterparts, so neither side means something else by them.

export type Device =
    | { state: "ready"; udid: string; name: string; version: string }
    | { state: "waiting"; udid: string; need: Need }

/// What a person must do before a phone will talk to us.
export type Need =
    | { need: "trust" }
    | { need: "unlock" }
    | { need: "replug" }
    | { need: "failed"; reason: string }

/// Where the photos come from. It changes on its own, so it is one field of
/// the view rather than the shape of it.
export type Source =
    | { source: "unavailable"; reason: string }
    | { source: "missing" }
    /// `chosen` is null while a copy winds down or a report waits for a phone
    /// that is not among them.
    | { source: "listed"; devices: Device[]; chosen: Device | null }

/// The chosen phone's camera roll: being read, or read.
export type Roll =
    | {
          roll: "reading"
          seen: number
          /// The photo just found, by path on the phone.
          latest: string | null
          /// Why the last walk failed. It is tried again until one gets through.
          failed: string | null
      }
    | {
          roll: "done"
          /// What the folder's takes and the range let through. Its newest
          /// are the prints of a screen with nothing new.
          all: Batch
          /// Those of them the folder has never taken.
          new: Batch
      }

/// What could not be copied. One bad file does not end a run.
export type Trouble = { failed: number; name: string; reason: string }

/// What a run did, and how it ended.
export type Report = {
    imported: number
    /// What was actually fetched, and how long it took to fetch it.
    bytes: number
    seconds: number
    trouble: Trouble | null
    /// The newest few it copied, by path on the phone.
    newest: string[]
    /// The phone it copied from: the prints are that phone's.
    udid: string
} & (
    | { end: "done" }
    | { end: "stopped" }
    /// Stopped before the next file would leave the disk too full.
    | { end: "full" }
    /// The phone left during the copy.
    | { end: "unplugged" }
    | { end: "failed"; reason: string }
)

/// The import while it runs, then what it did.
export type Run =
    | {
          run: "copying"
          done: number
          total: number
          bytes: number
          totalBytes: number
          /// Seconds left, or null while the rate is not worth quoting.
          eta: number | null
          /// The file that landed last, by path on the phone.
          latest: string | null
      }
    | ({ run: "finished" } & Report)

/// The destination, and how far we have got with it. A folder's own answers
/// live inside the one variant that has them, so they can never be defaulted.
export type Folder =
    /// `suggested` is ~/Pictures/iPhone, where the system has Pictures.
    | { folder: "unset"; suggested: string | null }
    | { folder: "opening" }
    | { folder: "broken"; path: string; reason: string }
    | {
          folder: "known"
          path: string
          /// Files here were named with the layout, so it cannot change.
          frozen: boolean
          /// When it last took anything, in seconds. Null until it has.
          lastImport: number | null
          /// Room an import may use on its disk. Huge when the disk will not say.
          free: number
          network: boolean
          layout: string
          naming: string
          /// What it takes in. One answer, because "photos only, but bring the
          /// Live Photo videos" cannot mean anything.
          takes: Takes
      }

/// Files to import, and what they are. A Live Photo's clip is neither a photo
/// nor a video, so the three kinds add up to the files.
export type Batch = {
    files: number
    bytes: number
    videos: number
    clips: number
    /// The newest few, by path on the phone, newest first. Never a Live
    /// Photo's clip.
    newest: string[]
}

/// How far back this run reaches. `last` keeps counting from today; `between`
/// is two fixed days.
export type Range =
    | { range: "everything" }
    | { range: "last"; days: number }
    | { range: "between"; from: string; to: string }

export type Takes = "everything" | "noLiveVideos" | "photos" | "videos"

/// The facts Rust holds, side by side. What to show is worked out from them
/// in `screen`.
export type Flow = {
    source: Source
    folder: Folder
    roll: Roll | null
    run: Run | null
    range: Range
}

/// Nothing plugged in and nothing chosen: what the window shows before Rust
/// has said anything.
export const QUIET: Flow = {
    source: { source: "missing" },
    folder: { folder: "unset", suggested: null },
    roll: null,
    run: null,
    range: { range: "everything" },
}

/// One thing the window is waiting for, said once: a drawing of what it is
/// about, a title, one instruction, and what the machine said.
export type Said = {
    art: "phone" | "lock" | "folder" | "trouble"
    title: string
    detail: string
    /// What the machine said, under what a person can act on.
    reason: string
    /// Something the app keeps doing by itself, said in a small pill.
    status: string | null
    /// A link to Apple Devices in the Microsoft Store, beside Troubleshooting.
    store?: true
    /// The Troubleshooting answer to open, instead of the page's top.
    answer?: "linux-usbmuxd"
}

/// What the window shows. `setup` is drawn by Setup, before there is a
/// folder; every other screen by Sync.
export type Screen =
    | {
          screen: "setup"
          said: Said
          /// The folder step's button, when there is one to press, and a
          /// folder to offer before the dialog, as the window names it.
          offer: null | { suggested: string | null }
      }
    /// The phone will not talk to us yet, or its camera roll will not open.
    | { screen: "prompt"; said: Said }
    | { screen: "counting"; count: string; caption: string }
    | { screen: "current"; detail: string }
    | {
          screen: "ready"
          /// Numbers, not words: the window counts from one to the next when
          /// Options change them.
          files: number
          bytes: number
          details: string
          /// Worth saying before Import, but not worth refusing over. Short:
          /// the run will stop when the disk is too full; a note is advice.
          warning: { tone: "short" | "note"; text: string } | null
      }
    | {
          screen: "importing"
          percent: number
          caption: string
          progress: string
          left: string
      }
    | {
          screen: "done"
          title: string
          detail: string
          /// What the machine said, when the run failed or a file would not
          /// copy.
          reason: string
          badge: "done" | "stopped" | "full" | "unplugged" | "trouble"
          /// The one button: go on with what is left, or look again once
          /// nothing is left. "counting" is Continue, shown but not yet
          /// usable, while what is left is counted.
          next: "continue" | "counting" | "rescan" | null
          /// What to do now, under the buttons.
          after: string
      }

/// The answers worth one click. Anything else is two dates.
export const PRESETS: { range: Range; label: string }[] = [
    { range: { range: "everything" }, label: "Any date" },
    { range: { range: "last", days: 7 }, label: "Last 7 days" },
    { range: { range: "last", days: 30 }, label: "Last 30 days" },
    { range: { range: "last", days: 365 }, label: "Last 12 months" },
]

/// What a folder takes in, as the window says it. A Live Photo's .mov is part
/// of a picture rather than a film of its own, so only the first of these
/// brings them along.
export const TAKES: Record<Takes, string> = {
    everything: "All media",
    noLiveVideos: "No Live Photo videos",
    photos: "Photos only",
    videos: "Videos only",
}

/// The footer's summary of the Options: "Any date, all media". The media types
/// lowercase their first letter, after the date.
export function summary(range: Range, takes: Takes) {
    const what = TAKES[takes]
    return `${when(range)}, ${what.charAt(0).toLowerCase()}${what.slice(1)}`
}

/// A range as one string, because that is what a menu can carry.
export function keyOf(range: Range) {
    return range.range === "last" ? `last:${range.days}` : range.range
}

/// A range as the window says it. Two dates read as the days they are.
export function when(range: Range) {
    return range.range === "between"
        ? span(range.from, range.to)
        : (PRESETS.find((preset) => keyOf(preset.range) === keyOf(range))
              ?.label ?? "")
}

/// What a file can be called. A folder keeps whichever it was created with.
export const LAYOUTS = [
    {
        layout: "{mtime:%Y-%m-%d_%H-%M-%S}_{name}",
        name: "Date and time in the name",
        example: "2026-09-02_15-04-11_IMG_0001.HEIC",
    },
    {
        layout: "{mtime:%Y}/{mtime:%m}/{name}",
        name: "Year and month folders",
        example: "2026/09/IMG_0001.HEIC",
    },
    {
        layout: "{name}",
        name: "The phone’s own name",
        example: "IMG_0001.HEIC",
    },
]

/// Up to date: what the folder already has of what the iPhone shows.
function here(n: number) {
    return n === 0
        ? "The iPhone has no photos or videos for these dates and media types."
        : n === 1
          ? "The one photo or video is already here."
          : `All ${count(n)} photos and videos are already here.`
}

/// A span that has already happened, as a person would say it.
export function elapsed(seconds: number) {
    const unit = (n: number, one: string) => `${n} ${n === 1 ? one : `${one}s`}`

    if (seconds < 60) return unit(Math.max(seconds, 1), "second")

    const minutes = Math.round(seconds / 60)

    if (minutes < 60) return unit(minutes, "minute")

    const hours = Math.floor(minutes / 60)
    const rest = minutes % 60

    return rest === 0
        ? unit(hours, "hour")
        : `${unit(hours, "hour")} ${unit(rest, "minute")}`
}

/// How long is left, said the way a person would say it. Under a minute it is
/// not worth a number; past that it is the same phrase as a span that has
/// already happened, which is why that one does the spelling.
export function duration(seconds: number) {
    return seconds < 45
        ? "Less than a minute left"
        : `About ${elapsed(Math.max(seconds, 60))} left`
}

/// A day as a person would name it, for saying how long it has been.
export function since(seconds: number) {
    const when = new Date(seconds * 1000)
    const midnight = (d: Date) =>
        new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime()

    const today = new Date()
    const days = Math.round((midnight(today) - midnight(when)) / 86400000)

    if (days <= 0) return "today"
    if (days === 1) return "yesterday"

    return when.toLocaleDateString(undefined, {
        day: "numeric",
        month: "long",
        // The year only when it is not this one; "4 March 2025" says something
        // "4 March" does not, and every other day it is noise.
        ...(when.getFullYear() === today.getFullYear()
            ? {}
            : { year: "numeric" }),
    })
}

/// Two days as one phrase. Intl knows how a range is said here — "4–11 Sep"
/// in one place, "Sep 4 – 11" in another — and doing it by hand produced
/// "4–September 11".
export function span(from: string, to: string) {
    const start = new Date(`${from}T00:00:00`)
    const end = new Date(`${to}T00:00:00`)
    const thisYear = new Date().getFullYear()

    // The year only when it is not this one; it says nothing every other day.
    const shape: Intl.DateTimeFormatOptions = {
        day: "numeric",
        month: "short",
        ...(start.getFullYear() === thisYear && end.getFullYear() === thisYear
            ? {}
            : { year: "numeric" }),
    }

    return new Intl.DateTimeFormat(undefined, shape).formatRange(start, end)
}

export function count(n: number) {
    return n.toLocaleString()
}

export function size(bytes: number) {
    const units = ["B", "KB", "MB", "GB", "TB"]
    // One decimal below 10 of a unit, as it rounds: 9.96 MB is "10 MB".
    const shown = (n: number, unit: number) =>
        n.toFixed(n < 9.95 && unit > 0 ? 1 : 0)
    let n = bytes
    let unit = 0

    // The next unit once the number would show as 1000: "1.0 MB", not
    // "1000 KB".
    while (Number(shown(n, unit)) >= 1000 && unit < units.length - 1) {
        n /= 1000
        unit++
    }

    // A no-break space: "1.7" and "GB" never land on two lines.
    return `${shown(n, unit)}\u00A0${units[unit]}`
}

/// Ask Rust to do something. Nothing here waits for an answer — the answer
/// arrives as the next flow — but a command that could not be delivered at
/// all should say so somewhere rather than as an unhandled rejection.
export function tell(command: string, args?: Record<string, unknown>) {
    invoke(command, args).catch((e) => console.error(command, e))
}

/// A folder as the window names it: the home folder as ~, and a long path
/// without its start, so the folder's own name is the part that stays.
export function place(path: string) {
    const short = path.replace(/^\/(Users|home)\/[^/]+(?=\/|$)/, "~")
    if (short.length <= 24) return short

    const sep = short.includes("\\") ? "\\" : "/"
    const parts = short.split(sep)
    let tail = parts.pop() ?? short

    while (parts.length && tail.length + parts.at(-1)!.length < 22)
        tail = `${parts.pop()}${sep}${tail}`

    return `…${sep}${tail}`
}

/// ⌘ and Finder on a Mac; Ctrl and Explorer on Windows; Ctrl and the
/// system's file manager on Linux, which has no one name for it.
export const mac = navigator.userAgent.includes("Mac")
export const linux = !mac && navigator.userAgent.includes("Linux")

type Waiting = "unavailable" | "missing" | Need["need"]

// One instruction for each thing the phone waits for. Every one of them
// clears by itself once it is done, so none of them has a button.
const WAITING: Record<Waiting, Omit<Said, "reason">> = {
    unavailable: {
        art: "trouble",
        title: "Cannot see iPhones",
        detail: mac
            ? "Unplug the cable and plug it in again. If that does not help, restart the Mac."
            : linux
              ? "The iPhone is plugged in, but usbmuxd does not answer. Restart usbmuxd, or install it if it is missing."
              : "Open Apple Devices, then plug the iPhone in again. If you do not have it, get it from the Microsoft Store.",
        status: "Checking again every few seconds",
        ...(!mac && !linux && { store: true }),
        // Commands are typed from the help, where they can be copied.
        ...(linux && { answer: "linux-usbmuxd" as const }),
    },
    missing: {
        art: "phone",
        title: "Connect your iPhone",
        // What comes next as well: a locked iPhone that has not trusted this
        // computer does not show up at all, so this is the only screen to say it.
        detail: "Plug it in with a USB cable and unlock it. If it asks whether to trust this computer, tap Trust.",
        status: "Looking for an iPhone",
    },
    trust: {
        art: "lock",
        title: "Tap Trust on the iPhone",
        detail: "The iPhone asks whether to trust this computer. Tap Trust, then enter your passcode.",
        status: "Waiting for the iPhone",
    },
    unlock: {
        art: "lock",
        title: "Unlock the iPhone",
        detail: "Unlock it with your passcode. It then asks whether to trust this computer.",
        status: "Waiting for the iPhone",
    },
    replug: {
        art: "phone",
        title: "Trust was refused",
        detail: "Unplug the iPhone and plug it in again. Then tap Trust when it asks.",
        status: null,
    },
    failed: {
        art: "trouble",
        title: "Cannot connect to the iPhone",
        detail: mac
            ? "Unplug the iPhone, unlock it, and plug it in again. Rollport keeps trying."
            : "Unplug the iPhone, unlock it, and plug it in again. If that does not help, restart the computer.",
        status: "Checking again every few seconds",
    },
}

/// What the phone needs before it will talk to us, in words, or null once it
/// talks to us.
export function needs(source: Source): Said | null {
    switch (source.source) {
        case "unavailable":
            // On Windows the reason names usbmuxd, which nobody there knows.
            return {
                ...WAITING.unavailable,
                reason: mac || linux ? source.reason : "",
            }
        case "missing":
            return { ...WAITING.missing, reason: "" }
        case "listed": {
            if (!source.chosen) return { ...WAITING.missing, reason: "" }
            if (source.chosen.state === "ready") return null

            const need = source.chosen.need
            return {
                ...WAITING[need.need],
                reason: need.need === "failed" ? need.reason : "",
            }
        }
    }
}

/// What the window shows, worked out from the facts. The first match wins.
export function screen(flow: Flow): Screen {
    const { folder, roll, run } = flow
    const need = needs(flow.source)

    // A copy only starts into a known folder. One that cannot be read after
    // the run is the thing to fix: the report's buttons need the folder.
    if (folder.folder !== "known") {
        if (need)
            return {
                screen: "setup",
                said: need,
                offer: null,
            }

        return folderStep(folder)
    }

    if (run?.run === "copying") {
        return {
            screen: "importing",
            percent: Math.floor(
                (run.bytes / Math.max(run.totalBytes, 1)) * 100,
            ),
            caption: run.eta === null ? "Importing…" : duration(run.eta),
            progress: `${count(run.done)} of ${count(run.total)}`,
            left: `${size(Math.max(run.totalBytes - run.bytes, 0))} left`,
        }
    }

    // What the last run did outranks what the cable is doing now: pulling the
    // cable is the first thing anybody does once a copy finishes.
    if (run?.run === "finished")
        return report(run, flow.source, roll, folder.free)

    if (need) return { screen: "prompt", said: need }

    // A locked phone answers lockdown but not its files. Rust keeps asking,
    // so unlocking it is the whole of what to do: the calm lock screen, not a
    // failure. Any other reason the roll will not open is one.
    if (roll?.roll === "reading" && roll.failed !== null) {
        if (/locked|passwordprotected/i.test(roll.failed))
            return {
                screen: "prompt",
                said: {
                    art: "lock",
                    title: "Unlock the iPhone",
                    detail: "Rollport reads the camera roll as soon as it is unlocked.",
                    reason: "",
                    status: "Waiting for the iPhone",
                },
            }
        return {
            screen: "prompt",
            said: {
                art: "trouble",
                title: "Cannot read the camera roll",
                detail: "Unlock the iPhone. Rollport keeps trying.",
                reason: roll.failed,
                status: "Checking again every few seconds",
            },
        }
    }

    if (roll?.roll !== "done") {
        return roll
            ? {
                  screen: "counting",
                  count: count(roll.seen),
                  caption: "photos and videos found",
              }
            : {
                  screen: "counting",
                  count: "0",
                  caption: "Looking through the camera roll",
              }
    }

    if (roll.new.files === 0) {
        const last = folder.lastImport

        return {
            screen: "current",
            detail: `${here(roll.all.files)}${last === null ? "" : ` Last import ${since(last)}.`}`,
        }
    }

    // "312 / 4.1 GB to import": what the button will do, in its own word,
    // and what it weighs. A full disk is said first: it will stop the run.
    return {
        screen: "ready",
        files: roll.new.files,
        bytes: roll.new.bytes,
        details: kinds(roll.new),
        warning:
            roll.new.bytes > folder.free
                ? {
                      tone: "short",
                      text: `Free up ${size(roll.new.bytes - folder.free)}, or choose fewer dates or media types in Options.`,
                  }
                : folder.network
                  ? {
                        tone: "note",
                        text: "On a network drive, a dropped connection stops the import. You can go on later.",
                    }
                  : null,
    }
}

/// No folder to import into yet, or one that will not open.
function folderStep(
    folder: Exclude<Folder, { folder: "known" }>,
): Extract<Screen, { screen: "setup" }> {
    const broken = folder.folder === "broken"

    return {
        screen: "setup",
        said: {
            art: broken ? "trouble" : "folder",
            title: broken ? "Cannot use this folder" : "Choose where photos go",
            detail: broken
                ? `Choose another folder, or check that ${place(folder.path)} is there and you can write to it.`
                : "Each import adds the new photos and videos here.",
            status: folder.folder === "opening" ? "Opening the folder" : null,
            reason: broken ? folder.reason : "",
        },
        // A folder that would not open gets the dialog, not the same offer.
        offer:
            folder.folder === "opening"
                ? null
                : {
                      suggested:
                          folder.folder === "unset" ? folder.suggested : null,
                  },
    }
}

/// What a report's phone needs before Continue, while it is connected.
const GO_ON: Record<Need["need"], string> = {
    trust: "Tap Trust on the iPhone to continue.",
    unlock: "Unlock the iPhone to continue.",
    replug: "Unplug the iPhone and plug it in again to continue.",
    failed: "Cannot connect to the iPhone. Rollport keeps trying.",
}

/// What the last run did: a short title, then what happened and what it
/// cost, then what is left or what went wrong; and the one thing to do next.
function report(
    run: Report,
    source: Source,
    roll: Roll | null,
    free: number,
): Extract<Screen, { screen: "done" }> {
    const { imported, trouble, bytes, seconds } = run
    const left = roll?.roll === "done" ? roll.new.files : null
    const chosen = source.source === "listed" ? source.chosen : null
    const ready = chosen?.state === "ready"
    // A locked phone answers, but not with its files.
    const unreadable = roll?.roll === "reading" && roll.failed !== null
    // After a run that copied nothing, the button starts it again.
    const goOn =
        imported > 0 ? "Continue imports what is left." : "Import starts again."
    // A run that did not do all of it goes on once its phone can be read.
    const unfinished = run.end !== "done" || trouble !== null
    const cost = imported > 0 ? `${size(bytes)} in ${elapsed(seconds)}. ` : ""

    const next: Pick<
        Extract<Screen, { screen: "done" }>,
        "next" | "after"
    > = ready && left === null
        ? {
              // A run that did not do all of it most likely leaves some.
              next: unfinished ? "counting" : null,
              after: unreadable
                  ? unfinished
                      ? "Unlock the iPhone to continue."
                      : "Unlock the iPhone to look for new photos."
                  : unfinished
                    ? goOn
                    : "Looking through the camera roll…",
          }
        : ready && left
          ? {
                next: "continue",
                after: goOn,
            }
          : ready
            ? { next: "rescan", after: "You can unplug the iPhone." }
            : chosen?.state === "waiting" && unfinished
              ? { next: null, after: GO_ON[chosen.need.need] }
              : unfinished
                ? { next: null, after: "Connect the iPhone again to continue." }
                : chosen
                  ? { next: null, after: "You can unplug the iPhone." }
                  : {
                        next: null,
                        after: "Connect the iPhone to look for new photos.",
                    }

    // Another import holds the folder (phone.rs, lock_folder): nothing went
    // wrong, so it is a wait, not a failure.
    if (
        run.end === "failed" &&
        run.reason.startsWith("Another import is writing")
    ) {
        return {
            title: "Another import is running",
            detail: "It is writing into this folder now. Import again once it is done.",
            reason: "",
            badge: "stopped",
            screen: "done",
            ...next,
        }
    }

    if (run.end === "failed") {
        return {
            title: "Import failed",
            detail:
                imported > 0
                    ? "Everything imported before it failed is still in the folder."
                    : "Nothing was copied.",
            reason: run.reason,
            badge: "trouble",
            screen: "done",
            ...next,
        }
    }

    // What fits is in; the rest needs room first.
    if (run.end === "full") {
        const short = roll?.roll === "done" ? roll.new.bytes - free : 0

        return {
            title:
                imported > 0
                    ? `Imported ${count(imported)}`
                    : "Not enough space",
            detail: `${cost}Free up ${short > 0 ? size(short) : "space"} to import${imported > 0 ? " the rest" : ""}.`,
            reason: "",
            badge: "full",
            screen: "done",
            ...next,
        }
    }

    // A run somebody stopped, or whose phone left, says how much of it is
    // left, because that is what Continue will do.
    const rest =
        left === null
            ? ready && !unreadable
                ? "Counting what is left…"
                : ""
            : left
              ? `${count(left)} left to import.`
              : "Nothing left to import."

    if (run.end === "unplugged") {
        return {
            title:
                imported > 0
                    ? `Disconnected after ${count(imported)}`
                    : "iPhone disconnected",
            detail: (cost + rest).trim(),
            reason: "",
            badge: "unplugged",
            screen: "done",
            ...next,
        }
    }

    if (run.end === "stopped") {
        return {
            title:
                imported > 0 ? `Stopped after ${count(imported)}` : "Stopped",
            detail: (cost + rest).trim(),
            reason: "",
            badge: trouble ? "trouble" : "stopped",
            screen: "done",
            ...next,
        }
    }

    // A run that lost files says which one and what it said: the difference
    // between one bad photo and a phone that locked halfway.
    if (trouble) {
        const { failed, name, reason } = trouble
        const upper = (said: string) =>
            said.charAt(0).toUpperCase() + said.slice(1)
        const which =
            failed === 1
                ? `${name ? upper(name) : "A photo or video"} could not be copied and is still on the iPhone.`
                : `${count(failed)} photos or videos could not be copied and are still on the iPhone.${name ? ` The first was ${name}.` : ""}`

        return {
            title:
                imported > 0
                    ? `Imported ${count(imported)}`
                    : "Nothing could be imported",
            detail: `${cost}${which}`,
            reason,
            badge: "trouble",
            screen: "done",
            ...next,
        }
    }

    return {
        ...(imported > 0
            ? {
                  title: `Imported ${count(imported)}`,
                  detail: `${cost}Nothing was deleted from the iPhone.`,
              }
            : {
                  title: "Nothing to import",
                  detail: "This folder already had every photo and video, so nothing was copied.",
              }),
        reason: "",
        badge: "done",
        screen: "done",
        ...next,
    }
}

/// What is new, by kind: "298 photos, 14 videos". Its weight is in the
/// caption above, so this line stays one line.
function kinds(batch: Batch) {
    const photos = batch.files - batch.videos - batch.clips
    const said = (n: number, one: string, many: string) =>
        n > 0 ? `${count(n)} ${n === 1 ? one : many}` : ""

    return [
        said(photos, "photo", "photos"),
        said(batch.videos, "video", "videos"),
        said(batch.clips, "Live Photo video", "Live Photo videos"),
    ]
        .filter(Boolean)
        .join(", ")
}
