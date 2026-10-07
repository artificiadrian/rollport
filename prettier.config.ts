import type { Config } from "prettier"

const config: Config = {
    semi: false,
    tabWidth: 4,
    // prettier-plugin-tailwindcss must stay last.
    plugins: [
        "prettier-plugin-svelte",
        "prettier-plugin-astro",
        "prettier-plugin-tailwindcss",
    ],
    tailwindStylesheet: "./src/app.css",
    overrides: [
        { files: "*.svelte", options: { parser: "svelte" } },
        { files: "*.astro", options: { parser: "astro" } },
        // YAML at 2 spaces, as GitHub's own workflow examples are written.
        { files: ["*.yml", "*.yaml"], options: { tabWidth: 2 } },
        // The site's classes sort against the site's own stylesheet.
        {
            files: "site/**",
            options: { tailwindStylesheet: "./site/src/styles/global.css" },
        },
    ],
}

export default config
