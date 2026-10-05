<script lang="ts">
    import { tick } from "svelte"
    import { invoke } from "@tauri-apps/api/core"
    import ChevronDown from "@lucide/svelte/icons/chevron-down"
    import Lock from "@lucide/svelte/icons/lock"
    import Menu from "$lib/components/Menu.svelte"
    import {
        LAYOUTS,
        PRESETS,
        TAKES,
        keyOf,
        tell,
        when,
        type Folder,
        type Range,
        type Takes,
    } from "$lib/flow"

    // What the popover under Options holds: which dates this run takes, and
    // the two answers the folder keeps in its rollport-library.db — what it takes in and
    // what it names files. Three rows, each a menu, but the file names once
    // files here carry them; every choice applies as it is made.
    let {
        folder,
        range,
    }: {
        folder: Extract<Folder, { folder: "known" }>
        range: Range
    } = $props()

    const onrange = (range: Range) => tell("set_range", { range })

    /// Rust answers a refused pattern with why, in words to show as they are.
    async function onlayout(layout: string) {
        try {
            await invoke("set_layout", { layout })
        } catch (reason) {
            trouble = String(reason)
        }
    }

    const id = $props.id()

    const from = $derived(range.range === "between" ? range.from : daysAgo(7))
    const to = $derived(range.range === "between" ? range.to : daysAgo(0))

    function pickDate(value: string) {
        if (value === "between") {
            onrange({ range: "between", from, to })
            return
        }

        const preset = PRESETS.find((preset) => keyOf(preset.range) === value)

        if (preset) onrange(preset.range)
    }

    // Either date on its own is half an answer, so the other one comes along.
    function edge(which: "from" | "to", value: string) {
        if (!value) return

        onrange({
            range: "between",
            from: which === "from" ? value : from,
            to: which === "to" ? value : to,
        })
    }

    /// A day this many days back, as the calendar here reads it. toISOString()
    /// is UTC, which east of Greenwich hands back yesterday all evening.
    function daysAgo(days: number) {
        const day = new Date()
        day.setDate(day.getDate() - days)

        return [
            day.getFullYear(),
            String(day.getMonth() + 1).padStart(2, "0"),
            String(day.getDate()).padStart(2, "0"),
        ].join("-")
    }

    /// The one value no layout can have — a layout has to print {name} — so the
    /// menu can carry an answer that is not a layout yet.
    const OWN = "own"

    let field = $state<HTMLInputElement>()

    /// Choosing "a pattern of your own" is a decision in itself, and can be
    /// made while a preset is still in force. Until a pattern is typed, that
    /// preset is what names files and Rust is not told otherwise.
    let asked = $state(false)

    /// Why the pattern in the field is not one this folder can keep, if it is
    /// not. Empty is the ordinary case.
    let trouble = $state("")

    /// The named layout the folder uses, if it is one of ours.
    const preset = $derived(
        LAYOUTS.find((option) => option.layout === folder.layout),
    )

    const own = $derived(
        asked || !LAYOUTS.some((option) => option.layout === folder.layout),
    )

    async function pickLayout(value: string) {
        trouble = ""

        if (value !== OWN) {
            asked = false
            onlayout(value)
            return
        }

        asked = true
        // The field is only drawn once the answer is "own".
        await tick()
        field?.focus()
        field?.select()
    }

    function commit(next: string) {
        const pattern = next.trim()

        // Nothing typed: the preset still in force is the answer.
        if (!pattern) {
            trouble = ""
            asked = false
            return
        }

        // A refused pattern stays in the field, where the correction is made.
        onlayout(pattern)
    }

    const row = "relative flex h-10 items-center gap-3 px-4"
    // The ring sits inside the row: the panel scrolls, and clips anything
    // drawn outside it. Important, to win over the ring in design.css, which
    // is outside Tailwind's layers.
    const trigger = `${row} w-full text-left transition-colors -outline-offset-2! hover:bg-subtle focus-visible:bg-subtle data-[state=open]:bg-subtle`

    const DATES = [
        ...PRESETS.map((preset) => ({
            value: keyOf(preset.range),
            label: preset.label,
        })),
        { value: "between", label: "Between two dates" },
    ]

    // What a choice leaves in or out, where its name alone does not say.
    const KIND_DETAILS: Partial<Record<Takes, string>> = {
        everything: "Photos, videos and Live Photo videos",
        noLiveVideos: "The short clip with each Live Photo is left out",
    }

    const KINDS = Object.entries(TAKES).map(([value, label]) => ({
        value,
        label,
        detail: KIND_DETAILS[value as Takes],
    }))

    const NAMES = [
        ...LAYOUTS.map((option) => ({
            value: option.layout,
            label: option.name,
            detail: option.example,
            mono: true,
        })),
        { value: OWN, label: "A pattern of your own", detail: ownDetail },
    ]

    const input =
        "h-7 min-w-0 cursor-text rounded-md border border-line bg-subtle px-2 select-text transition-colors hover:border-edge"
</script>

{#snippet ownDetail()}
    You write it, for example <span class="font-mono"
        >{"{mtime:%Y}/{name}"}</span
    >
{/snippet}

{#snippet answer(value: string, mono = false)}
    <span class="ml-auto flex min-w-0 items-center gap-2 font-medium">
        <span class="truncate {mono ? 'font-mono text-xs' : ''}">{value}</span>
        <ChevronDown size={12} class="shrink-0 text-muted" aria-hidden="true" />
    </span>
{/snippet}

<div class="divide-y divide-line">
    <!-- A row and the fields it opens are one group: no line between them. -->
    <div>
        <Menu
            value={keyOf(range)}
            items={DATES}
            onchoose={pickDate}
            label="Date taken, not saved with the folder"
            title="Not saved with the folder. Back to Any date when Rollport is next opened."
            class={trigger}
        >
            <span class="text-muted" aria-hidden="true">Date taken</span>
            <!-- "Between two dates" is how you ask for two dates; it is not what
                 the answer says once you have. -->
            {@render answer(when(range))}
        </Menu>

        {#if range.range === "between"}
            <div class="flex items-center gap-2 px-4 pb-3 text-xs tabular-nums">
                <input
                    type="date"
                    value={from}
                    max={to}
                    aria-label="First day"
                    onchange={(event) =>
                        edge("from", event.currentTarget.value)}
                    class="{input} flex-1"
                />
                <span class="shrink-0 text-muted">to</span>
                <input
                    type="date"
                    value={to}
                    min={from}
                    aria-label="Last day"
                    onchange={(event) => edge("to", event.currentTarget.value)}
                    class="{input} flex-1"
                />
            </div>
        {/if}
    </div>

    <Menu
        value={folder.takes}
        items={KINDS}
        onchoose={(value) => tell("set_takes", { takes: value as Takes })}
        label="Media types, saved with this folder"
        title="Saved with this folder."
        class={trigger}
    >
        <span class="text-muted" aria-hidden="true">Media types</span>
        {@render answer(TAKES[folder.takes])}
    </Menu>

    <!-- The naming stopped being a question when the folder named its first
         file. It is a thing that happened, not a control that was turned off. -->
    {#if folder.frozen}
        <div
            class={row}
            title="Set at the first import and cannot change now, so every file here is named the same way."
        >
            <span class="text-muted">File names</span>
            <span class="ml-auto flex min-w-0 items-center gap-2">
                <!-- A pattern of its own has no name, so it shows itself. -->
                {#if preset}
                    <span class="truncate font-medium">{preset.name}</span>
                {:else}
                    <span class="truncate font-mono text-xs select-text"
                        >{folder.layout}</span
                    >
                {/if}
                <Lock
                    size={12}
                    class="shrink-0 text-muted"
                    aria-hidden="true"
                />
                <span class="sr-only"
                    >, locked: set at the first import, so every file here is
                    named the same way</span
                >
            </span>
        </div>
    {:else}
        <div>
            <Menu
                value={own ? OWN : folder.layout}
                items={NAMES}
                onchoose={pickLayout}
                label="File names, saved with this folder"
                title="Saved with this folder. Set for good at the first import."
                class={trigger}
            >
                <span class="text-muted" aria-hidden="true">File names</span>
                {@render answer(
                    own ? "A pattern of your own" : (preset?.name ?? ""),
                )}
            </Menu>

            {#if own}
                <!-- The field is positioned, so its focus ring paints over the
                     highlighted row above it, which is positioned too. -->
                <div class="px-4 pb-3">
                    <input
                        bind:this={field}
                        type="text"
                        spellcheck="false"
                        autocapitalize="off"
                        autocorrect="off"
                        value={folder.layout}
                        aria-label="Naming pattern"
                        aria-invalid={trouble ? "true" : undefined}
                        aria-describedby="{id}-help"
                        oninput={() => (trouble = "")}
                        onchange={(event) => commit(event.currentTarget.value)}
                        class="{input} relative w-full font-mono text-xs"
                    />
                    <p
                        id="{id}-help"
                        class="mt-2 text-xs text-pretty {trouble
                            ? 'text-tomato-11'
                            : 'text-muted'}"
                    >
                        {#if trouble}
                            <!-- Its tokens set as code, as in the hint. -->
                            {#each trouble.split(/(\{[^}]*\})/) as part, index (index)}
                                {#if part.startsWith("{")}
                                    <span class="font-mono whitespace-nowrap"
                                        >{part}</span
                                    >
                                {:else}
                                    {part}
                                {/if}
                            {/each}
                        {:else}
                            <!-- A token never breaks at its hyphens. -->
                            <span class="font-mono whitespace-nowrap"
                                >{"{name}"}</span
                            >
                            is the iPhone’s name for the file,
                            <span class="font-mono whitespace-nowrap"
                                >{"{mtime:%Y-%m-%d}"}</span
                            >
                            is the date it was taken, and
                            <span class="font-mono">/</span> makes a folder.
                        {/if}
                    </p>
                </div>
            {/if}
        </div>
    {/if}
</div>
