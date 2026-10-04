<script lang="ts">
    import type { HTMLButtonAttributes } from "svelte/elements"

    // The screen's actions. The main one is solid ink, raised above the
    // others, which are frosted over the photo's tint (glass, in design.css).
    // +page.svelte gives the focus to .btn-main when the screen changes.
    let {
        variant,
        size = "small",
        class: extra,
        children,
        ...rest
    }: HTMLButtonAttributes &
        (
            | {
                  variant: "main"
                  /// Large for the screen's one action, medium on setup,
                  /// small beside a secondary.
                  size?: "large" | "medium" | "small"
              }
            | { variant: "secondary"; size?: "small" }
        ) = $props()

    const VARIANT = {
        main: "btn-main",
        secondary: "btn-glass glass",
    }

    const SIZE = {
        large: "h-11 rounded-2xl px-6 text-sm",
        medium: "h-10 rounded-xl px-6 text-sm",
        small: "h-9 rounded-xl px-4",
    }
</script>

<button
    type="button"
    {...rest}
    class={[
        "flex items-center gap-2 font-medium",
        VARIANT[variant],
        SIZE[size],
        extra,
    ]}
>
    {@render children?.()}
</button>
