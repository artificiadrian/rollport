<script lang="ts">
    import { convertFileSrc } from "@tauri-apps/api/core"
    import { cubicOut } from "svelte/easing"
    import { Tween, prefersReducedMotion } from "svelte/motion"
    import { fade, fly } from "svelte/transition"
    import ArrowDownToLine from "@lucide/svelte/icons/arrow-down-to-line"
    import ArrowRight from "@lucide/svelte/icons/arrow-right"
    import Check from "@lucide/svelte/icons/check"
    import ChevronDown from "@lucide/svelte/icons/chevron-down"
    import FolderIcon from "@lucide/svelte/icons/folder"
    import FolderOpen from "@lucide/svelte/icons/folder-open"
    import ImageIcon from "@lucide/svelte/icons/image"
    import RotateCw from "@lucide/svelte/icons/rotate-cw"
    import SlidersHorizontal from "@lucide/svelte/icons/sliders-horizontal"
    import Smartphone from "@lucide/svelte/icons/smartphone"
    import Pause from "@lucide/svelte/icons/pause"
    import Unplug from "@lucide/svelte/icons/unplug"
    import TriangleAlert from "@lucide/svelte/icons/triangle-alert"
    import HardDrive from "@lucide/svelte/icons/hard-drive"
    import Button from "$lib/components/Button.svelte"
    import Footer from "$lib/components/Footer.svelte"
    import HelpLink from "$lib/components/HelpLink.svelte"
    import Menu from "$lib/components/Menu.svelte"
    import Options from "$lib/components/Options.svelte"
    import Prompt from "$lib/components/Prompt.svelte"
    import Reason from "$lib/components/Reason.svelte"
    import {
        count,
        linux,
        mac,
        place,
        size,
        summary,
        tell,
        type Device,
        type Flow,
        type Folder,
        type Screen,
    } from "$lib/flow"

    // The screen every run after the first opens on: where from, where to,
    // and one thing in the middle — how many are new, how far the copy is,
    // what it did, or that there is nothing to do.
    let {
        view,
        flow,
        folder,
    }: {
        view: Exclude<Screen, { screen: "setup" }>
        flow: Flow
        /// `flow.folder`, known: this screen is only drawn with one.
        folder: Extract<Folder, { folder: "known" }>
    } = $props()

    /// The phone the window is about, ready or not.
    const shown = $derived(
        flow.source.source === "listed" ? flow.source.chosen : null,
    )

    /// The phone the pill names, once it talks to us. Until then the pill
    /// says what it waits for, dashed.
    const chosen = $derived(shown?.state === "ready" ? shown : null)

    /// A phone not ready has no name yet. In the menu, the end of its udid
    /// tells two apart; the pill says only "iPhone".
    const named = (device: Device) =>
        device.state === "ready"
            ? device.name
            : `iPhone ${device.udid.slice(-4)}`

    /// The phone pill while there is no phone to name.
    const UNSEEN = {
        unavailable: "Cannot see iPhones",
        missing: "Connect your iPhone",
        listed: "iPhone",
    }

    const devices = $derived(
        flow.source.source === "listed" ? flow.source.devices : [],
    )

    /// A picture is its phone and its path there: two phones each have an
    /// IMG_0001, and the window must not take one for the other.
    const of = (udid: string | undefined) => (path: string) =>
        `${udid ?? ""}${path}`

    /// The newest of what is to import, front first.
    const newest = $derived(
        flow.roll?.roll === "done"
            ? flow.roll.new.newest.map(of(shown?.udid))
            : [],
    )

    /// The photo that arrived last, while files are being copied or the
    /// camera roll is being counted. Null otherwise: a report read again
    /// under keeps the prints of its run.
    const latest = $derived.by(() => {
        const path =
            flow.run?.run === "copying"
                ? flow.run.latest
                : view.screen === "counting" && flow.roll?.roll === "reading"
                  ? flow.roll.latest
                  : null
        return path && shown ? of(shown.udid)(path) : null
    })

    /// Options is open. How the window looks, so it is this screen's to keep.
    let options = $state(false)

    /// The button that opens Options. Closed with Esc or a click outside, the
    /// popover takes the focus with it; it comes back here, not to the top.
    let opener = $state<HTMLButtonElement>()
    let wasOpen = false

    $effect(() => {
        const open = options
        if (wasOpen && !open && opener?.isConnected) {
            const lost = document.activeElement
            if (!lost || lost === document.body || !lost.isConnected)
                opener.focus()
        }
        wasOpen = open
    })

    /// Start the import. Options close, so they do not come back by
    /// themselves when the run is over.
    function start() {
        options = false
        tell("start_import")
    }

    function shortcut(event: KeyboardEvent) {
        // An open menu takes Esc first and marks it handled; it closes alone.
        if (event.key === "Escape" && !event.defaultPrevented) options = false

        // A pattern being typed has not been given to Rust yet, and ⌘⏎ would
        // start the import with the layout the field is about to replace.
        if (event.target instanceof HTMLInputElement) return

        // ⌘ on a Mac, Ctrl on the machines this also runs on.
        if (!(mac ? event.metaKey : event.ctrlKey)) return

        // The screen's main button, when it shows one.
        if (event.key === "Enter") {
            if (
                view.screen === "ready" ||
                (view.screen === "done" && view.next === "continue")
            ) {
                event.preventDefault()
                start()
            } else if (view.screen === "done" && view.next === "rescan") {
                event.preventDefault()
                tell("rescan")
            }
        }

        // The one thing you want the keyboard for while a copy runs.
        if (event.key === "." && busy) {
            event.preventDefault()
            tell("cancel_import")
        }
    }

    const thumb = (path: string) => convertFileSrc(path, "thumb")

    // The prints once the import starts, by place: front, left, right. They
    // begin as the ready screen's, and while files land one is dealt onto the
    // front every so often, at a pace a person can look at rather than the
    // pace of the copy. What lands in between is not shown; the count says it.
    let dealt = $state<(string | undefined)[]>([])

    /// The side holding the older print, which is the next to go.
    let oldest: 1 | 2 = 2

    /// How often a print is dealt: time to see each photo, with the fan at
    /// rest about two thirds of it (a deal takes 350 ms).
    const DEAL = 1000

    const resting = $derived(view.screen === "ready")

    $effect(() => {
        if (resting) {
            dealt = newest
            oldest = 2
        }
    })

    /// The prints' photos, by path and by place: front, left, right. A print
    /// without one, or whose thumbnail will not load, is drawn as paper.
    /// A screen the window dealt nothing on keeps what it shows to hand: a
    /// report, the newest photos its run copied; otherwise, and when it copied
    /// none, the newest photos the folder already has.
    const pictures = $derived.by(() => {
        if (resting) return newest
        if (dealt.some(Boolean)) return dealt
        if (flow.run?.run === "finished" && flow.run.newest.length)
            return flow.run.newest.map(of(flow.run.udid))
        return flow.roll?.roll === "done"
            ? flow.roll.all.newest.map(of(shown?.udid))
            : []
    })

    /// Each picture's width over its height once it has loaded, or "broken"
    /// when the phone has no thumbnail for it. Missing while it is on its way.
    /// A print takes its photo's shape; until then, and with no photo, it is
    /// paper. Forgotten with the window.
    let seen = $state<Record<string, number | "broken">>({})

    /// The phone the window was last about, and the last that was there.
    let was: string | null = null
    let last: string | null = null

    // A phone that answers again may have the thumbnails that failed while
    // none did. Another phone's prints are not this one's to keep.
    $effect(() => {
        const udid = shown?.udid ?? null
        if (udid === was) return
        was = udid
        if (udid === null) return

        for (const key in seen) if (seen[key] === "broken") delete seen[key]
        if (last !== null && last !== udid) dealt = []
        last = udid
    })

    /// A print is waiting while its photo is on its way, or while the camera
    /// roll is still being read and no photo has been dealt to it yet.
    function waiting(url: string | undefined) {
        return url ? seen[url] === undefined : view.screen === "counting"
    }
    const PAPER = 3 / 4

    /// The front print's box; the side prints are smaller.
    const FRONT = 140

    // The fan, back to front: a card each side, tilted from a pivot below the
    // stack, and the newest upright in front. `side` is -1, 1, or 0 for front.
    const FAN = [
        { index: 1, side: -1, turn: -11, box: 116 },
        { index: 2, side: 1, turn: 11, box: 116 },
        // A little off true, as a print dropped on a table lands.
        { index: 0, side: 0, turn: -2, box: FRONT },
    ]

    /// How much of a side print shows past the front one. The rest tucks
    /// under it, so the three always read as one stack, whatever their shapes.
    const SHOWN = 0.55

    /// How far a side print sits from the middle: its inner part under the
    /// front print, the rest showing.
    function shift(card: (typeof FAN)[number], width: number) {
        if (card.side === 0) return 0

        const front = fit(pictures[0], FRONT).width

        return card.side * (front / 2 + width * (SHOWN - 1 / 2))
    }

    function shapeOf(url: string, image: HTMLImageElement) {
        seen[url] = image.naturalWidth / Math.max(image.naturalHeight, 1)
    }

    /// Load and measure a picture before it is dealt, so its print lands in
    /// its own shape. One with no thumbnail is dealt all the same, as paper.
    async function measure(url: string) {
        if (seen[url] !== undefined) return

        const image = new Image()
        image.src = thumb(url)

        try {
            await image.decode()
            shapeOf(url, image)
        } catch {
            // No thumbnail: the print stays paper.
            seen[url] = "broken"
        }
    }

    function deal(picture: string) {
        if (dealt.includes(picture)) return

        const next = [dealt[0], dealt[1], dealt[2]]

        if (next[0] === undefined) {
            next[0] = picture
        } else {
            // The front print takes the place of the oldest side one, so no
            // print crosses the stack to get where it is going.
            const out =
                next[1] === undefined ? 1 : next[2] === undefined ? 2 : oldest

            next[out] = next[0]
            next[0] = picture
            oldest = out === 1 ? 2 : 1
        }

        dealt = next
    }

    /// A deal is on its way. The next tick waits for it rather than asking
    /// the phone for the same thumbnail twice, and prints land in order.
    let dealing = false

    // `latest` is read inside the callback, so the timer is not restarted
    // each time it changes; it is null while nothing is moving.
    $effect(() => {
        const timer = setInterval(() => {
            const picture = latest

            if (!picture || dealing || dealt.includes(picture)) return

            dealing = true
            void measure(picture)
                .then(() => deal(picture))
                .finally(() => (dealing = false))
        }, DEAL)

        return () => clearInterval(timer)
    })

    /// A print's size: its photo's shape, as large as fits a square box.
    /// Panoramas and tall screenshots are held to 2:1, or they would be slivers.
    function fit(url: string | undefined, box: number) {
        const known = url ? seen[url] : undefined
        const measured = typeof known === "number" ? known : PAPER
        const shape = Math.min(Math.max(measured, 1 / 2), 2)

        return shape >= 1
            ? { width: box, height: box / shape }
            : { width: box * shape, height: box }
    }

    const still = $derived(prefersReducedMotion.current)

    // On the ready screen, the count and the size run to their new values when
    // Options change them. Arriving on the screen shows them at once: a count
    // from the last screen's number would mean nothing.
    const files = new Tween(0, { easing: cubicOut })
    const bytes = new Tween(0, { easing: cubicOut })
    let wasReady = false
    $effect(() => {
        if (view.screen !== "ready") {
            wasReady = false
            return
        }
        const duration = wasReady && !still ? 300 : 0
        files.set(view.files, { duration })
        bytes.set(view.bytes, { duration })
        wasReady = true
    })

    /// The last run copied nothing, so its button starts it again rather than
    /// going on.
    const again = $derived(
        flow.run?.run === "finished" && flow.run.imported === 0,
    )

    // Nothing about the route can change while files are moving along it.
    const busy = $derived(view.screen === "importing")

    /// A menu whenever there is another phone to choose, so a phone that never
    /// gets ready can be left for one that is.
    const menu = $derived(
        !busy && devices.some((device) => device.udid !== shown?.udid),
    )

    // Frosted over the photo's tint (glass, in design.css).
    const pill =
        "glass flex h-8 max-w-48 items-center gap-2 rounded-full px-3 font-medium"

    /// The shortcuts, as a screen reader is told them, and as the tooltips
    /// show them.
    const enter = mac ? "Meta+Enter" : "Control+Enter"
    const period = mac ? "Meta+." : "Control+."
    const shownEnter = mac ? "⌘⏎" : "Ctrl+Enter"
    const shownPeriod = mac ? "⌘." : "Ctrl+."

    const hero =
        "mt-3 text-6xl font-bold tracking-tight tabular-nums dark:[text-shadow:0_2px_18px_rgb(0_0_0/0.35)]"

    const heading = "mt-3 text-2xl font-semibold"

    const id = $props.id()
</script>

{#snippet phone(name: string)}
    <Smartphone size={14} class="shrink-0 text-muted" aria-hidden="true" />
    <span class="truncate">{name}</span>
{/snippet}

<div class="relative flex min-h-0 grow flex-col overflow-hidden">
    <!-- The front photo, blurred past recognising, tints the window: the
         space around the prints is theirs, not an empty floor. It follows
         the front print, so while importing it changes with each deal. -->
    {#if pictures[0] && seen[pictures[0]] !== "broken"}
        {#key pictures[0]}
            <!-- The wrapper fades; the blurred picture inside stays on its own
                 layer throughout. Fading the blurred picture itself made
                 WebKit redraw it brighter the moment the fade ended. -->
            <div
                aria-hidden="true"
                transition:fade={{ duration: still ? 0 : 600 }}
                class="pointer-events-none absolute inset-0 opacity-20 dark:opacity-25 plain:hidden"
                style="will-change: opacity; mask-image: linear-gradient(to bottom, transparent 0%, black 22%, black 42%, transparent 85%)"
            >
                <img
                    src={thumb(pictures[0])}
                    alt=""
                    class="absolute -inset-16 h-[calc(100%+8rem)] w-[calc(100%+8rem)] max-w-none object-cover blur-[60px] saturate-150"
                    style="transform: translateZ(0)"
                />
            </div>
        {/key}
    {/if}

    <!-- Where from and where to. Each end is the place to change it. -->
    <div
        class="relative flex items-center justify-center gap-2 px-6 pt-3 {busy
            ? 'text-muted'
            : ''}"
        inert={busy}
    >
        <!-- Dashed while it waits: it will fill in by itself. With a phone to
             choose other than the one shown, the pill is a menu of them. -->
        {#if menu}
            <Menu
                value={shown?.udid ?? ""}
                items={devices.map((device) => ({
                    value: device.udid,
                    label: named(device),
                    detail:
                        device.state === "ready"
                            ? `iOS ${device.version}`
                            : "Not ready",
                }))}
                onchoose={(udid) => tell("choose_device", { udid })}
                label="iPhone to import from"
                class="btn-glass {pill} {chosen
                    ? ''
                    : 'border-dashed! border-edge! bg-transparent! text-muted'}"
            >
                {@render phone(chosen?.name ?? UNSEEN.listed)}
                <ChevronDown
                    size={12}
                    class="shrink-0 text-muted"
                    aria-hidden="true"
                />
            </Menu>
        {:else}
            <span
                class="{pill} {chosen
                    ? ''
                    : 'border-dashed! border-edge! bg-transparent! text-muted'}"
            >
                {@render phone(chosen?.name ?? UNSEEN[flow.source.source])}
            </span>
        {/if}

        <ArrowRight size={14} class="shrink-0 text-faint" aria-hidden="true" />

        <button
            type="button"
            onclick={() => tell("choose_destination")}
            title={`${folder.path}\nChoose another folder`}
            class="btn-glass {pill}"
        >
            <FolderIcon
                size={14}
                class="shrink-0 text-muted"
                aria-hidden="true"
            />
            <span class="truncate">{place(folder.path)}</span>
        </button>
    </div>

    <!-- Every screen is centred between two spacers that grow alike. The
         warning sits in the lower one: it does not move the rest, and when it
         needs more room than that spacer has, flexbox takes it from the upper
         one. The prints slide between screens in a view transition. -->
    <div
        data-stage
        tabindex="-1"
        class="relative flex grow flex-col items-center px-8 text-center outline-none"
    >
        <div class="min-h-6 flex-1"></div>
        <div class="flex w-full shrink-0 flex-col items-center">
            {#if view.screen === "prompt"}
                <Prompt said={view.said} />
            {:else}
                <!-- A few prints, fanned like a hand of cards: this is a photo
                 tool before it is a number. Without a photo, a print is drawn
                 as paper, not as a grey box that passes for one. Keyed by
                 picture, so a print that stays slides to its new place, a new
                 one is dealt in from above, and one leaving fades. -->
                <div
                    class="relative h-[164px] w-[340px] [view-transition-name:prints]"
                    aria-hidden="true"
                >
                    <!-- Back to front, each keyed by its picture so it keeps its element
     when it moves to another place in the fan. -->
                    {#each FAN as card (pictures[card.index] ?? `paper-${card.index}`)}
                        {@const url = pictures[card.index]}
                        {@const size = fit(url, card.box)}
                        <div
                            in:fly={{ y: -28, duration: still ? 0 : 350 }}
                            out:fade={{ duration: still ? 0 : 250 }}
                            class="absolute bottom-3 left-1/2 rounded-lg border border-edge bg-canvas p-[5px]
                            transition-[width,height,transform] duration-300 ease-out motion-reduce:transition-none
                            {card.index === 0
                                ? 'shadow-[0_14px_30px_-6px_rgb(0_0_0/0.28)] dark:shadow-[0_16px_34px_-6px_rgb(0_0_0/0.6)]'
                                : 'shadow-[0_6px_16px_-4px_rgb(0_0_0/0.2)] dark:shadow-[0_8px_20px_-4px_rgb(0_0_0/0.5)]'}"
                            style="width: {size.width +
                                12}px; height: {size.height +
                                12}px; transform-origin: 50% 160%; transform: translateX(-50%) translateX({shift(
                                card,
                                size.width,
                            )}px) rotate({card.turn}deg)"
                        >
                            <!-- Paper first, the photo over it once it has loaded. Paper
                             shimmers while a photo is on its way, and names the file
                             when the phone has no thumbnail for it. -->
                            <div
                                class="grid h-full place-items-center overflow-hidden rounded border border-line
                                {waiting(url)
                                    ? 'animate-shimmer bg-[linear-gradient(100deg,transparent_30%,var(--element)_50%,transparent_70%)] bg-[length:250%_100%] motion-reduce:animate-none'
                                    : ''}"
                            >
                                {#if url && seen[url] === "broken"}
                                    {@const name = url.split("/").pop() ?? ""}
                                    {@const dot = name.lastIndexOf(".")}
                                    <!-- Two lines at most, broken before the extension. -->
                                    <span
                                        class="line-clamp-2 max-w-full px-2 text-center font-mono text-[10px] text-muted"
                                    >
                                        {dot > 0
                                            ? name.slice(0, dot)
                                            : name}<wbr />{dot > 0
                                            ? name.slice(dot)
                                            : ""}
                                    </span>
                                {:else if card.index === 0 && !waiting(url)}
                                    <ImageIcon
                                        size={28}
                                        strokeWidth={1.25}
                                        class="text-faint"
                                    />
                                {/if}
                            </div>
                            {#if url && seen[url] !== "broken"}
                                <img
                                    src={thumb(url)}
                                    alt=""
                                    onload={(event) =>
                                        shapeOf(
                                            url,
                                            event.currentTarget as HTMLImageElement,
                                        )}
                                    onerror={() => (seen[url] = "broken")}
                                    class="absolute inset-[5px] h-[calc(100%-10px)] w-[calc(100%-10px)] rounded-sm object-cover transition-opacity duration-200
                                    {seen[url] === undefined
                                        ? 'opacity-0'
                                        : 'opacity-100'}"
                                />
                            {/if}

                            <!-- What happened, pinned to the front print's corner. -->
                            {#if card.index === 0 && (view.screen === "current" || view.screen === "done")}
                                <span
                                    class="absolute -right-4 -bottom-4 grid size-10 place-items-center rounded-full border-3 border-canvas
                                    {view.screen !== 'done'
                                        ? 'bg-ink text-canvas'
                                        : view.badge === 'trouble'
                                          ? 'bg-tomato-9 text-white'
                                          : view.badge === 'full'
                                            ? 'bg-orange-9 text-white'
                                            : 'bg-ink text-canvas'}"
                                >
                                    {#if view.screen === "current" || view.badge === "done"}
                                        <Check size={18} strokeWidth={3} />
                                    {:else if view.badge === "stopped"}
                                        <Pause
                                            size={14}
                                            strokeWidth={0}
                                            fill="currentColor"
                                        />
                                    {:else if view.badge === "unplugged"}
                                        <Unplug size={17} strokeWidth={2.5} />
                                    {:else if view.badge === "full"}
                                        <!-- A full disk, not a failure: its own shape, not only
                                             another colour. -->
                                        <HardDrive
                                            size={17}
                                            strokeWidth={2.5}
                                        />
                                    {:else}
                                        <TriangleAlert
                                            size={17}
                                            strokeWidth={2.5}
                                        />
                                    {/if}
                                </span>
                            {/if}
                        </div>
                    {/each}
                </div>
                {#if view.screen === "ready"}
                    <p class={hero}>{count(Math.round(files.current))}</p>
                    <p class="mt-2 text-lg font-medium">
                        {size(bytes.current)} to import
                    </p>
                    {#key view.details}
                        <p
                            class="mt-2 text-muted"
                            in:fade={{ duration: still ? 0 : 200 }}
                        >
                            {view.details}
                        </p>
                    {/key}

                    <Button
                        variant="main"
                        size="large"
                        onclick={start}
                        title={`Import (${shownEnter})`}
                        aria-keyshortcuts={enter}
                        class="mt-6"
                    >
                        <ArrowDownToLine
                            size={16}
                            strokeWidth={2}
                            aria-hidden="true"
                        />
                        Import {count(view.files)}
                    </Button>
                {:else if view.screen === "counting"}
                    <p class="{hero} text-muted">{view.count}</p>
                    <p class="mt-2 text-lg font-medium">{view.caption}</p>
                    <!-- Where the button will be, so nothing jumps when it arrives. -->
                    <div class="mt-6 h-11"></div>
                {:else if view.screen === "importing"}
                    <p class={hero}>{view.percent}%</p>
                    <p class="mt-2 text-lg font-medium">{view.caption}</p>

                    <div
                        role="progressbar"
                        aria-valuemin={0}
                        aria-valuemax={100}
                        aria-valuenow={view.percent}
                        aria-valuetext="{view.percent}%, {view.caption}"
                        class="mt-4 h-2 w-full overflow-hidden rounded-full bg-ink/12 forced-colors:border forced-colors:border-[CanvasText]"
                    >
                        <div
                            class="h-full rounded-full bg-ink transition-[width] duration-300 motion-reduce:transition-none forced-colors:bg-[CanvasText]"
                            style="width: {view.percent}%"
                        ></div>
                    </div>
                    <div
                        class="mt-2 flex w-full justify-between text-xs text-muted tabular-nums"
                    >
                        <span>{view.progress}</span>
                        <span>{view.left}</span>
                    </div>

                    <Button
                        variant="secondary"
                        onclick={() => tell("cancel_import")}
                        title={`Stop (${shownPeriod})`}
                        aria-keyshortcuts={period}
                        class="mt-6"
                    >
                        Stop
                    </Button>
                {:else if view.screen === "done"}
                    <p class={heading}>{view.title}</p>
                    <p class="mt-2 text-sm text-balance text-muted">
                        {view.detail}
                    </p>
                    {#if view.reason}
                        <Reason reason={view.reason} />
                    {/if}
                    {#if view.badge === "trouble"}
                        <HelpLink page="troubleshooting"
                            >Troubleshooting</HelpLink
                        >
                    {/if}
                    <div class="mt-6 flex items-center gap-2">
                        {#if view.next === "continue" || view.next === "counting"}
                            <Button
                                variant="main"
                                onclick={start}
                                disabled={view.next === "counting"}
                                class="disabled:opacity-50"
                                title={`${again ? "Import again" : "Import what is left"} (${shownEnter})`}
                                aria-keyshortcuts={enter}
                            >
                                <ArrowDownToLine
                                    size={14}
                                    strokeWidth={2}
                                    aria-hidden="true"
                                />
                                {again ? "Import" : "Continue"}
                            </Button>
                        {:else if view.next === "rescan"}
                            <Button
                                variant="main"
                                onclick={() => tell("rescan")}
                                title={`Check the camera roll again (${shownEnter})`}
                                aria-keyshortcuts={enter}
                            >
                                <RotateCw size={14} aria-hidden="true" />
                                Check again
                            </Button>
                        {/if}
                        <Button
                            variant="secondary"
                            onclick={() => tell("reveal_destination")}
                        >
                            <FolderOpen size={14} aria-hidden="true" />
                            {mac
                                ? "Show in Finder"
                                : linux
                                  ? "Show in file manager"
                                  : "Show in Explorer"}
                        </Button>
                    </div>
                {:else}
                    <p class={heading}>Up to date</p>
                    <p class="mt-2 text-sm text-muted">
                        {view.detail}
                    </p>
                    <Button
                        variant="secondary"
                        onclick={() => tell("rescan")}
                        class="mt-6"
                    >
                        <RotateCw size={14} aria-hidden="true" />
                        Check again
                    </Button>
                {/if}
            {/if}
        </div>
        <div class="flex w-full flex-1 flex-col items-center">
            {#if view.screen === "ready" && view.warning}
                <!-- Said, not refused: it is their disk and their decision. -->
                <!-- The icon sits in the text, so balanced lines centre with it. -->
                <p
                    class="my-4 text-sm text-balance {view.warning.tone ===
                    'short'
                        ? 'font-medium text-orange-11'
                        : 'text-muted'}"
                >
                    {#if view.warning.tone === "short"}
                        <HardDrive
                            size={16}
                            class="mr-1 inline align-[-3px]"
                            aria-hidden="true"
                        />
                    {/if}{view.warning.text}
                </p>
            {/if}
        </div>
    </div>
</div>

<svelte:window onkeydown={shortcut} />

<!-- pl-7 matches the help button and its gap on the right, so a centred
     sentence centres on the window. -->
<Footer variant="status">
    {#if view.screen === "importing"}
        <span class="grow pl-7 text-center">
            Stop keeps what is copied. The next import goes on from there.
        </span>
    {:else if view.screen === "done"}
        <span class="grow pl-7 text-center">{view.after}</span>
    {:else}
        <button
            bind:this={opener}
            type="button"
            onclick={() => (options = !options)}
            aria-expanded={options}
            aria-controls="{id}-options"
            class="-ml-2 flex h-6 items-center gap-2 rounded-md px-2 font-medium text-ink transition-colors hover:bg-element active:bg-fill"
        >
            <SlidersHorizontal size={14} aria-hidden="true" />
            Options
            <ChevronDown
                size={12}
                aria-hidden="true"
                class={options ? "rotate-180" : ""}
            />
        </button>
        <span class="grow truncate text-right"
            >{summary(flow.range, folder.takes)}</span
        >
    {/if}
</Footer>

<!-- Options float over the window rather than growing it, and close on a
     click outside or Esc. The window keeps its size. After the footer, so Tab
     goes from the Options button into them. -->
{#if options}
    <button
        type="button"
        aria-label="Close options"
        tabindex="-1"
        onclick={() => (options = false)}
        transition:fade={{ duration: still ? 0 : 150 }}
        class="fixed inset-0 z-40 cursor-default bg-canvas/70"
    ></button>
    <!-- 8 px above the footer (h-12), and 12 px below the window top at most. -->
    <div
        role="dialog"
        id="{id}-options"
        aria-label="Options"
        transition:fly={{ y: 8, duration: still ? 0 : 150 }}
        class="fixed right-3 bottom-14 left-3 z-50 max-h-[calc(100vh-var(--spacing)*17)] overflow-y-auto rounded-xl border border-line bg-canvas shadow-xl"
    >
        <Options {folder} range={flow.range} />
    </div>
{/if}
