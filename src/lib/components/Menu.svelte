<script lang="ts">
    import type { Snippet } from "svelte"
    import { Select } from "bits-ui"
    import Check from "@lucide/svelte/icons/check"

    // A menu of answers, drawn by the app: the system's menu is plain white
    // on Windows, whatever the theme. The trigger is the caller's to draw;
    // the list looks the same everywhere.
    let {
        value,
        items,
        onchoose,
        label,
        title,
        class: look,
        children,
    }: {
        value: string
        /// `detail` is a second, quieter line: words, or a snippet when part of
        /// it is code; `mono` sets all of it as code.
        items: {
            value: string
            label: string
            detail?: string | Snippet | undefined
            mono?: boolean
        }[]
        /// Called with the item chosen. The menu shows `value` until the
        /// caller's answer changes it.
        onchoose: (value: string) => void
        /// What a screen reader calls the trigger.
        label: string
        title?: string
        /// The trigger's classes.
        class: string
        /// What the trigger shows.
        children: Snippet
    } = $props()

    const id = $props.id()
</script>

<!-- Controlled: a choice Rust refuses must not stay ticked. -->
<Select.Root
    type="single"
    {items}
    bind:value={() => value, (next) => next && next !== value && onchoose(next)}
>
    <!-- A combobox, as the system's menu is: a screen reader then reads the
         answer the trigger shows after its label, and follows the
         highlighted item, which a button with a popup does not. -->
    <Select.Trigger
        role="combobox"
        aria-controls="{id}-list"
        aria-label={label}
        {title}
        class={look}
    >
        {@render children()}
    </Select.Trigger>
    <Select.Portal>
        <!-- Above the Options panel (z-50) that some of these sit in. -->
        <Select.Content
            id="{id}-list"
            aria-label={label}
            sideOffset={4}
            collisionPadding={8}
            class="z-60 max-h-(--bits-select-content-available-height) w-max max-w-(--bits-select-content-available-width) min-w-(--bits-select-anchor-width) overflow-y-auto rounded-lg border border-line bg-canvas p-1 text-sm shadow-lg transition-opacity duration-100 outline-none data-ending-style:opacity-0 data-starting-style:opacity-0 motion-reduce:transition-none"
        >
            {#each items as item (item.value)}
                <Select.Item
                    value={item.value}
                    label={item.label}
                    class="flex min-h-8 cursor-default items-start gap-2 rounded-md py-2 pr-4 pl-2 outline-none select-none data-highlighted:bg-element"
                >
                    {#snippet children({ selected })}
                        <!-- As tall as the label's line, so the tick centres on it. -->
                        <span class="flex h-5 shrink-0 items-center">
                            <Check
                                size={14}
                                aria-hidden="true"
                                class={selected ? "" : "invisible"}
                            />
                        </span>
                        <span class="flex min-w-0 flex-col">
                            <span class="truncate">{item.label}</span>
                            {#if item.detail}
                                <span
                                    class="truncate text-xs text-muted {item.mono
                                        ? 'font-mono'
                                        : ''}"
                                    >{#if typeof item.detail === "string"}{item.detail}{:else}{@render item.detail()}{/if}</span
                                >
                            {/if}
                        </span>
                    {/snippet}
                </Select.Item>
            {/each}
        </Select.Content>
    </Select.Portal>
</Select.Root>
