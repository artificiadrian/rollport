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
        multiple,
        ontick,
    }: {
        /// One answer, or with `multiple` every answer that is ticked: the
        /// list then stays open, and a click ticks or unticks.
        value?: string
        multiple?: string[]
        /// `detail` is a second, quieter line; `mono` sets it as code.
        items: {
            value: string
            label: string
            detail?: string | undefined
            mono?: boolean
            /// A picture of the answer, before its label.
            icon?: typeof Check
            /// A number or a word at the row's end, such as how many.
            aside?: string | undefined
            /// Set under the answer before it, as a part of it.
            indent?: boolean
            disabled?: boolean
        }[]
        /// Called with the item chosen. The menu shows `value` until the
        /// caller's answer changes it.
        onchoose?: (value: string) => void
        /// Called with every answer ticked, when `multiple` is given.
        ontick?: (values: string[]) => void
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
{#if multiple}
    <Select.Root
        type="multiple"
        {items}
        bind:value={() => multiple, (next) => ontick?.(next)}
    >
        {@render list()}
    </Select.Root>
{:else}
    <Select.Root
        type="single"
        {items}
        bind:value={
            () => value ?? "",
            (next) => next && next !== value && onchoose?.(next)
        }
    >
        {@render list()}
    </Select.Root>
{/if}

{#snippet list()}
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
        <!-- Above the Options panel (z-50) that some of these sit in. It comes
             and goes as that panel does: a fade, rising 8 px. -->
        <Select.Content
            id="{id}-list"
            aria-label={label}
            sideOffset={4}
            collisionPadding={8}
            class="z-60 max-h-(--bits-select-content-available-height) w-max max-w-(--bits-select-content-available-width) min-w-(--bits-select-anchor-width) overflow-y-auto rounded-lg border border-line bg-canvas p-1 text-sm shadow-lg transition-[opacity,translate] duration-150 ease-out outline-none data-ending-style:translate-y-2 data-ending-style:opacity-0 data-starting-style:translate-y-2 data-starting-style:opacity-0 motion-reduce:transition-none"
        >
            {#each items as item (item.value)}
                <Select.Item
                    value={item.value}
                    label={item.label}
                    disabled={item.disabled ?? false}
                    class={[
                        "flex min-h-8 cursor-default items-start gap-2 rounded-md py-2 pr-4 outline-none select-none data-disabled:opacity-50 data-highlighted:bg-element",
                        item.indent ? "pl-8" : "pl-2",
                    ]}
                >
                    {#snippet children({ selected })}
                        <!-- As tall as the label's line, so it centres on it.
                             Several answers are boxes; one answer, a dot. -->
                        <span class="flex h-5 shrink-0 items-center">
                            {#if multiple}
                                <span
                                    class={[
                                        "flex size-4 items-center justify-center rounded-sm border",
                                        selected
                                            ? "border-ink bg-ink text-canvas"
                                            : "border-edge",
                                    ]}
                                >
                                    {#if selected}
                                        <Check
                                            size={12}
                                            strokeWidth={3}
                                            aria-hidden="true"
                                        />
                                    {/if}
                                </span>
                            {:else}
                                <span
                                    class={[
                                        "flex size-4 items-center justify-center rounded-full border",
                                        selected ? "border-ink" : "border-edge",
                                    ]}
                                >
                                    {#if selected}
                                        <span class="size-2 rounded-full bg-ink"
                                        ></span>
                                    {/if}
                                </span>
                            {/if}
                        </span>
                        {#if item.icon}
                            <span
                                class="flex h-5 shrink-0 items-center text-muted"
                            >
                                <item.icon size={14} />
                            </span>
                        {/if}
                        <span class="flex min-w-0 flex-col">
                            <span class="truncate">{item.label}</span>
                            {#if item.detail}
                                <span
                                    class="truncate text-xs text-muted {item.mono
                                        ? 'font-mono'
                                        : ''}">{item.detail}</span
                                >
                            {/if}
                        </span>
                        {#if item.aside}
                            <span
                                class="ml-auto pl-6 text-xs leading-5 text-muted tabular-nums"
                                >{item.aside}</span
                            >
                        {/if}
                    {/snippet}
                </Select.Item>
            {/each}
        </Select.Content>
    </Select.Portal>
{/snippet}
