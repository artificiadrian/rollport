<script lang="ts">
    import FolderIcon from "@lucide/svelte/icons/folder"
    import Button from "$lib/components/Button.svelte"
    import Footer from "$lib/components/Footer.svelte"
    import Prompt from "$lib/components/Prompt.svelte"
    import { fitText } from "$lib/fit"
    import { places, tell, type Screen } from "$lib/flow"

    // What the window says before there is a folder: no phone to import from
    // yet, or no folder to import into.
    let { view }: { view: Extract<Screen, { screen: "setup" }> } = $props()
</script>

<!-- Centred under the same 28 px as Sync's screens; a screen too tall for
     that starts at the top. -->
<div
    data-stage
    tabindex="-1"
    class="flex min-h-0 grow flex-col items-center px-8 text-center outline-none"
>
    <div class="min-h-6 shrink-1 grow-0 basis-[28px]"></div>
    <div class="my-auto flex w-full shrink-0 flex-col items-center">
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
                    <span
                        class="grow truncate text-sm font-medium"
                        {@attach fitText(places(view.offer.suggested))}
                    ></span>
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
    </div>
</div>

<Footer variant="help" />
