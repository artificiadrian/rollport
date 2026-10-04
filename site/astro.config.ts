import sitemap from "@astrojs/sitemap"
import tailwindcss from "@tailwindcss/vite"
import { defineConfig } from "astro/config"
import { site } from "./src/site"

// https://docs.astro.build/en/reference/configuration-reference/
export default defineConfig({
    site: site.origin,
    base: site.base,
    integrations: [sitemap()],
    vite: { plugins: [tailwindcss()] },
})
