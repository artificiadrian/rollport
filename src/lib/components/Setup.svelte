<script lang="ts">
    import FolderIcon from "@lucide/svelte/icons/folder"
    import Button from "$lib/components/Button.svelte"
    import Footer from "$lib/components/Footer.svelte"
    import Prompt from "$lib/components/Prompt.svelte"
    import { place, tell, type Screen } from "$lib/flow"

    // What the window says before there is a folder: no phone to import from
    // yet, or no folder to import into.
    let { view }: { view: Extract<Screen, { screen: "setup" }> } = $props()
</script>

<!-- The art starts 72 px from the top, where Sync has it, so going from
     setup to the phone's screens does not move it; only a screen too tall for
     that takes room from above. -->
<div
    data-stage
    tabindex="-1"
    class="flex min-h-0 grow flex-col items-center px-8 text-center outline-none"
>
    <div class="min-h-6 shrink-1 grow-0 basis-[72px]"></div>
    <Prompt said={view.said}>
        {#if view.offer?.suggested}
            <!-- The folder to take: the suggested one, or one picked here or
             dropped on the window. The whole field picks another; only the
             button under it takes it. -->
            <button
                type="button"
                onclick={() => tell("choose_destination")}
                title={`${view.offer.suggested}\nChoose another folder`}
                class="group mt-4 flex h-11 w-full items-center gap-2 rounded-lg border border-line bg-subtle pr-2 pl-3 text-left transition-colors hover:border-edge"
            >
                <FolderIcon
                    size={16}
                    class="shrink-0 text-muted"
                    aria-hidden="true"
                />
                <span class="grow truncate text-sm font-medium">
                    {place(view.offer.suggested)}
                </span>
                <span
                    class="flex h-8 shrink-0 items-center rounded-md px-2 text-xs font-medium text-muted transition-colors group-hover:bg-element group-hover:text-ink group-active:bg-fill"
                >
                    Change…
                </span>
            </button>
            <Button
                variant="main"
                size="medium"
                onclick={() => tell("use_suggested_destination")}
                class="mt-4"
            >
                Use this folder
            </Button>
        {:else if view.offer}
            <Button
                variant="main"
                size="medium"
                onclick={() => tell("choose_destination")}
                class="mt-4"
            >
                <FolderIcon size={14} aria-hidden="true" />
                Choose folder…
            </Button>
            <p class="mt-2 text-xs text-muted">
                Or drop a folder on this window.
            </p>
        {/if}
    </Prompt>
    <div class="min-h-6 flex-1"></div>
</div>

<Footer variant="help" />
