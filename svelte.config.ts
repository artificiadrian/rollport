// Tauri doesn't have a Node.js server to do proper SSR
// so we use adapter-static with a fallback to index.html to put the site in SPA mode
// See: https://svelte.dev/docs/kit/single-page-apps
// See: https://v2.tauri.app/start/frontend/sveltekit/ for more info
import adapter from "@sveltejs/adapter-static"
import type { Config } from "@sveltejs/kit"
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte"

const config: Config = {
    preprocess: vitePreprocess(),
    compilerOptions: {
        runes: true,
    },
    kit: {
        adapter: adapter({
            fallback: "index.html",
        }),
        // The route is in the hash, not the path: the window has one route,
        // and the site's demo serves this page from /demo/index.html. It also
        // turns off server rendering, which Tauri has no server for.
        router: { type: "hash" },
    },
}

export default config
