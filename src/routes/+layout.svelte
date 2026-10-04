<script lang="ts">
    import "../app.css"

    let { children } = $props()
</script>

<!-- The window has one size, set in tauri.conf.json, and every screen fills
     it: a window that fits each screen jumps as the screens change. -->
<div class="h-screen w-full">
    {@render children()}
</div>
