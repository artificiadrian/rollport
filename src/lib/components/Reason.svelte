<script lang="ts">
    // Why something went wrong, in the words the OS used.
    let { reason }: { reason: string } = $props()
</script>

<!-- Selectable, so it can be searched for; clamped, because it is whatever
     the OS said and the window cannot scroll. -->
<p
    class="mt-2 line-clamp-2 cursor-text text-xs text-tomato-11 select-text"
    title={reason}
>
    <!-- A capital in code: ::first-letter does not reach into the clamp's box. -->
    {reason.charAt(0).toUpperCase() + reason.slice(1)}
</p>
