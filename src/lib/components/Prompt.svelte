<script lang="ts">
    import type { Snippet } from "svelte"
    import FolderIcon from "@lucide/svelte/icons/folder"
    import LoaderCircle from "@lucide/svelte/icons/loader-circle"
    import Lock from "@lucide/svelte/icons/lock"
    import TriangleAlert from "@lucide/svelte/icons/triangle-alert"
    import HelpLink from "$lib/components/HelpLink.svelte"
    import Reason from "$lib/components/Reason.svelte"
    import type { Said } from "$lib/flow"

    let {
        said,
        children,
    }: {
        said: Said
        /// What to press, when there is something. Never shown with a status:
        /// a status says the app is doing it by itself.
        children?: Snippet
    } = $props()
</script>

<!-- The thing it is about, in two faint rings: small enough that the title
     and the instruction lead. In a box as tall as Sync's prints, with the
     same 12 px under the art. -->
<div class="flex h-[164px] justify-center pt-4" aria-hidden="true" data-art>
    <div class="relative grid size-34 place-items-center">
        <!-- Red rings when something is wrong, in the muted reds borders use. -->
        <span
            class="absolute inset-0 rounded-full border {said.art === 'trouble'
                ? 'border-tomato-6'
                : 'border-fill'}"
        ></span>
        <span
            class="absolute inset-5 rounded-full border {said.art === 'trouble'
                ? 'border-tomato-7'
                : 'border-line'}"
        ></span>

        {#if said.art === "folder"}
            <FolderIcon
                size={50}
                strokeWidth={1.25}
                class="relative fill-canvas"
            />
        {:else if said.art === "trouble"}
            <TriangleAlert
                size={44}
                strokeWidth={1.25}
                class="relative fill-canvas text-tomato-11"
            />
        {:else}
            <div
                class="relative flex h-[76px] w-11 flex-col items-center rounded-[10px] border-2 border-ink bg-canvas pt-1.5"
            >
                <span class="h-[3px] w-3 rounded-sm bg-edge"></span>
                {#if said.art === "lock"}
                    <Lock size={17} strokeWidth={2} class="mt-4" />
                {/if}
            </div>
            <!-- The cable, from the bottom of the phone to the outer ring. -->
            <span
                class="absolute top-[106px] left-[67px] h-[30px] w-0.5 bg-faint"
            ></span>
        {/if}
    </div>
</div>

<p class="mt-3 text-2xl font-semibold">
    {said.title}
</p>
<p class="mt-2 text-sm text-muted">{said.detail}</p>

{#if said.reason}
    <Reason reason={said.reason} />
{/if}

{#if said.art === "trouble"}
    <div class="flex gap-3">
        {#if said.store}
            <HelpLink page="apple-devices">Get Apple Devices</HelpLink>
        {/if}
        <HelpLink page={said.answer ?? "troubleshooting"}>
            {said.answer === "linux-usbmuxd"
                ? "How to fix usbmuxd"
                : "Troubleshooting"}
        </HelpLink>
    </div>
{/if}

{@render children?.()}

{#if said.status}
    <span
        class="mt-6 flex h-6 items-center gap-2 rounded-full bg-element px-3 text-xs text-muted"
    >
        <!-- A turning spinner: a pulsing dot was too quiet to read as
             "still at it". -->
        <LoaderCircle
            size={12}
            strokeWidth={2.5}
            class="shrink-0 animate-spin text-ink motion-reduce:animate-none"
            aria-hidden="true"
        />
        {said.status}
    </span>
{/if}
