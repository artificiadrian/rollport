# Rollport site

The product page for Rollport. It is an Astro site.

The window at the top of the page is the real app interface. `scripts/demo.ts`
builds the app from the repository root, copies it to `public/demo/`, and
puts `demo/tauri.ts` in front of it. That file is a fake Rust side: an iPhone
with 312 new photos, and an import that takes six seconds. It uses the app's
`Flow` type, so `pnpm check` fails when the app changes its data and the demo
does not.

## Commands

Run these in `site/`.

```sh
pnpm dev       # build the demo, then serve the site at localhost:4321
pnpm build     # build the demo, then the site, into dist/
pnpm check     # type check, including demo/tauri.ts against the app
pnpm capture   # take the link preview image, public/og.png, while pnpm dev runs
```

`pnpm capture` needs Playwright's Chromium: `pnpm exec playwright install chromium`.

Links and the version number are in `src/site.ts`.
