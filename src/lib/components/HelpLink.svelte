<script lang="ts">
    import type { Snippet } from "svelte"
    import { tell } from "$lib/flow"

    // Under what went wrong: a page that helps, opened in the browser.
    let {
        page,
        children,
    }: {
        page: "troubleshooting" | "apple-devices" | "linux-usbmuxd"
        children: Snippet
    } = $props()
</script>

<button
    type="button"
    onclick={() => tell("show_help", { page })}
    class="mt-2 rounded px-1 text-xs text-muted underline decoration-faint underline-offset-2 transition-colors hover:text-ink hover:decoration-current"
>
    {@render children()}
</button>
