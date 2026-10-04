<script lang="ts">
    import type { Snippet } from "svelte"
    import { getVersion } from "@tauri-apps/api/app"
    import { tell } from "$lib/flow"

    // The row at the bottom of every screen, with the help at its end.
    let props:
        | {
              /// A bar: what the run is doing, or the way into Options.
              variant: "status"
              children: Snippet
          }
        | {
              /// Only the help, in the corner where the status bar has it; no
              /// bar, since there is nothing else to hold.
              variant: "help"
          } = $props()

    // The version, where a bug report can find it: there is no About window
    // on Windows or Linux.
    let version = $state("")
    getVersion()
        .then((answer) => (version = answer))
        .catch(() => {})
</script>

<footer
    class="flex h-12 shrink-0 items-center pr-3 {props.variant === 'status'
        ? 'justify-between gap-2 border-t border-line bg-subtle pl-4 text-xs text-muted'
        : 'justify-end'}"
>
    {#if props.variant === "status"}
        {@render props.children()}
    {/if}
    <!-- The help pages, opened in the browser. -->
    <button
        type="button"
        onclick={() => tell("show_help", { page: "help" })}
        aria-label="Help"
        title={version ? `Help (Rollport ${version})` : "Help"}
        class="grid size-5 shrink-0 place-items-center rounded-full bg-element font-semibold text-muted transition-colors hover:bg-fill hover:text-ink active:bg-edge"
    >
        ?
    </button>
</footer>
