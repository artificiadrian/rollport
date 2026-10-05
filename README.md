<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/mark-dark.svg">
    <img src="docs/mark.svg" width="128" alt="">
  </picture>
</p>

<h1 align="center">Rollport</h1>

<p align="center">
  Copy new photos and videos from your iPhone to a folder on your computer, over USB.<br>
  Nothing on the phone is deleted.
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/rollport-dark.webp">
    <img src="docs/rollport.webp" width="544" alt="The Rollport window: 312 new photos and videos, an import that deals them onto a fan of prints, and the report when it is done">
  </picture>
</p>

## Features

- Copies only new photos and videos, even after you delete or move old ones
- Original files: HEIC, ProRAW, Live Photos and video, never converted
- Sorted by date: dated file names, year and month folders, or a pattern of your own
- Choose a date range, and leave out videos, photos or Live Photo videos
- Stop at any time and continue later; unplugging during an import loses nothing
- Remembers each iPhone's folder; drop a folder on the window to switch
- Keyboard: <kbd>⌘</kbd>/<kbd>Ctrl</kbd>+<kbd>Enter</kbd> imports, <kbd>⌘</kbd>/<kbd>Ctrl</kbd>+<kbd>.</kbd> stops,
  <kbd>⌘</kbd>/<kbd>Ctrl</kbd>+<kbd>+</kbd>/<kbd>−</kbd>/<kbd>0</kbd> zooms, <kbd>⌘</kbd><kbd>?</kbd> or <kbd>F1</kbd> opens help
- Windows, macOS and Linux (Linux untested)
- Free and open source, no account, works offline

See the [help](https://artificiadrian.github.io/rollport/help) for everything
Rollport does.

## Download

No releases yet. To try it, [build it from source](#building).

- Windows: requires [Apple Devices](https://apps.microsoft.com/detail/9np83lwlpz9k)
  (Microsoft Store) or iTunes. The build isn't signed, so SmartScreen will warn
  you: More info → Run anyway. `Rollport.exe` runs without installing.
- macOS: the app isn't notarized, so macOS blocks it the first time. Allow it in
  System Settings → Privacy & Security.
- Linux: requires `usbmuxd` (`apt install usbmuxd` / `dnf install usbmuxd`).
  Untested.

The iPhone has to be unlocked. The first time you connect it, tap Trust on the
phone.

## Why I made this

<!-- Coming soon. -->

## Building

Requirements: [Rust](https://rustup.rs), [Node.js](https://nodejs.org) ≥ 22.18,
[pnpm](https://pnpm.io/installation), and the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

```sh
pnpm install
pnpm tauri dev
```

`pnpm tauri build` builds the installer. See
[CONTRIBUTING.md](CONTRIBUTING.md) for the rest.

## Licence

MIT

iPhone is a trademark of Apple Inc., registered in the U.S. and other
countries and regions. Rollport is not affiliated with or endorsed by Apple.
