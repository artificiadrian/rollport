<script lang="ts">
    import { invoke } from "@tauri-apps/api/core"
    import { listen } from "@tauri-apps/api/event"
    import { getCurrentWebview } from "@tauri-apps/api/webview"
    import { tick } from "svelte"
    import { prefersReducedMotion } from "svelte/motion"
    import { fade } from "svelte/transition"
    import FolderInput from "@lucide/svelte/icons/folder-input"
    import Setup from "$lib/components/Setup.svelte"
    import Sync from "$lib/components/Sync.svelte"
    import { QUIET, count, mac, screen, size, tell, type Flow } from "$lib/flow"

    let flow = $state<Flow>(QUIET)

    /// Where WebKitGTK aborts on a view transition; Rust says so before the
    /// page runs (lib.rs).
    const noTransitions =
        (window as { __ROLLPORT_NO_TRANSITIONS__?: boolean })
            .__ROLLPORT_NO_TRANSITIONS__ === true

    /// A folder is being dragged over the window. The whole window is the
    /// target: nobody drags a folder and then aims at a small rectangle.
    let dragging = $state(false)

    const view = $derived(screen(flow))

    /// A new flow. When it brings another screen, the browser animates the
    /// change: the prints slide to their new place and the rest cross-fades.
    /// Progress within a screen lands as it is, many times a second.
    function land(next: Flow) {
        // A transition on its way lands the newest flow, not the one that
        // started it: a later one must not be undone by an earlier.
        if (pending) {
            pending = next
            return
        }

        const other = screen(next).screen !== view.screen
        if (
            !other ||
            prefersReducedMotion.current ||
            !document.startViewTransition ||
            noTransitions
        ) {
            flow = next
            return
        }

        pending = next
        document.startViewTransition(async () => {
            flow = pending ?? next
            pending = null
            await tick()
        })
    }

    /// The newest flow while a view transition waits to draw it.
    let pending: Flow | null = null

    /// What had keyboard focus before the screen last changed.
    let focused: Element | null = null

    $effect.pre(() => {
        void view
        focused = document.activeElement
    })

    // A button that goes with its screen — Import, Stop, Continue — takes
    // the focus with it, and Tab would start again from the top. The focus
    // moves to the new screen's main button instead, or to the screen itself
    // when it has none. Focus that is elsewhere is left where it is.
    $effect(() => {
        void view
        if (!focused) return
        if (focused.isConnected && !focused.hasAttribute("data-stage")) return

        const stage = document.querySelector<HTMLElement>("[data-stage]")
        // A greyed button cannot take the focus, and it would stay nowhere.
        const target =
            stage?.querySelector<HTMLElement>(".btn-main:not(:disabled)") ??
            stage?.querySelector<HTMLElement>("button:not(:disabled)") ??
            stage

        target?.focus()
    })

    /// Parts of what the screen says, each a sentence, the empty ones left out.
    const sentences = (...parts: (string | null | undefined)[]) =>
        parts
            .filter(Boolean)
            .map((part) => (/[.!?…]$/.test(part!) ? part : `${part}.`))
            .join(" ")

    /// What a screen reader is told, at the moments worth telling: the same
    /// words the screen shows, so the two never say different things — the
    /// warning, the reason and what to do next with them.
    const announcement = $derived.by(() => {
        switch (view.screen) {
            case "setup":
            case "prompt":
                return sentences(
                    view.said.title,
                    view.said.detail,
                    view.said.reason,
                )
            // Every ten per cent, and nothing else: the time left changes
            // with the minute and would be read out each time.
            case "importing":
                return `${Math.floor(view.percent / 10) * 10}% imported`
            // "312 photos and videos, 4.1 GB to import": the big number
            // alone would run into its caption.
            case "ready":
                return sentences(
                    `${count(view.files)} ${view.files === 1 ? "photo or video" : "photos and videos"}, ${size(view.bytes)} to import`,
                    view.warning?.text,
                )
            case "done":
                return sentences(
                    view.title,
                    view.detail,
                    view.reason,
                    view.after,
                )
            case "current":
                return sentences("Up to date", view.detail)
            case "counting":
                return ""
        }
    })

    /// Ctrl with +, − and 0 sizes the window, and F1 opens the help. On a Mac
    /// the menus hold these, and take the keys before the page sees them.
    function keys(event: KeyboardEvent) {
        if (mac) return

        if (event.key === "F1") {
            event.preventDefault()
            tell("show_help", { page: "help" })
            return
        }

        if (!event.ctrlKey) return

        const step = { "=": "in", "+": "in", "-": "out", "0": "actual" }[
            event.key
        ]

        if (!step) return

        event.preventDefault()
        tell("zoom", { step })
    }

    // What the window listens to, for as long as it is here. Both subscriptions
    // are asked for asynchronously and can arrive after the window has gone, so
    // one keeps them and one lets them go.
    $effect(() => {
        let live = true
        const stops: (() => void)[] = []

        const keep = (asked: Promise<() => void>) =>
            asked
                .then((stop) => {
                    if (live) stops.push(stop)
                    else stop()
                })
                .catch((e) => console.error("listen", e))

        keep(
            // Only the overlay: Rust hears the drop itself and decides whether
            // what landed is a folder.
            getCurrentWebview().onDragDropEvent(({ payload: drag }) => {
                dragging = drag.type === "enter" || drag.type === "over"
            }),
        )

        let heard = false

        void keep(
            listen<Flow>("flow", (event) => {
                heard = true
                land(event.payload)
            }),
        )
            .then(() => invoke<Flow>("flow"))
            .then((first) => {
                // Rust worked that answer out before it published anything that
                // landed while it was on its way here. The newer one wins.
                if (!heard) flow = first
            })
            .catch((e) => console.error("flow", e))

        return () => {
            live = false
            stops.forEach((stop) => stop())
        }
    })
</script>

<svelte:window onkeydown={keys} />

<main class="flex h-full cursor-default flex-col text-sm select-none">
    {#if view.screen === "setup"}
        <Setup {view} />
    {:else if flow.folder.folder === "known"}
        <Sync {view} {flow} folder={flow.folder} />
    {/if}

    <p aria-live="polite" class="sr-only">{announcement}</p>

    <!-- Drag a folder anywhere onto the window and it becomes the one photos
         go to. The overlay is the only thing that says so, and it says it for
         exactly as long as the folder is over the window. -->
    {#if dragging && flow.run?.run !== "copying"}
        <div
            class="fixed inset-0 z-50 grid place-items-center bg-canvas p-6"
            transition:fade={{
                duration: prefersReducedMotion.current ? 0 : 100,
            }}
        >
            <div
                class="flex flex-col items-center gap-3 rounded-2xl border-2 border-dashed border-ink/55 px-8 py-8"
            >
                <FolderInput
                    size={36}
                    strokeWidth={1.5}
                    aria-hidden="true"
                    class="text-ink"
                />
                <p class="text-sm font-semibold">Drop a folder</p>
                <p class="text-muted">
                    Photos and videos are imported into it.
                </p>
            </div>
        </div>
    {/if}
</main>
