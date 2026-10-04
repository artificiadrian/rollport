# Contributing

[Tauri 2](https://tauri.app) + SvelteKit. The iPhone side uses the
[`idevice`](https://crates.io/crates/idevice) crate.

## What you need

- [Rust](https://rustup.rs) 1.85 or later
- [Node.js](https://nodejs.org) 22.18 or later (the scripts use Node's built-in
  TypeScript support)
- [pnpm](https://pnpm.io/installation) 11
- Tauri's [prerequisites](https://v2.tauri.app/start/prerequisites/): the Xcode
  Command Line Tools on a Mac, the Microsoft C++ Build Tools and WebView2 on
  Windows

## Commands

```sh
pnpm install
pnpm tauri dev      # run it
pnpm tauri build    # build it
pnpm screens        # every state of the window, photographed into screens/index.html
pnpm clip           # the README's clip, light and dark, into docs/ (needs brew install webp)
```

## Cross-compiling for Windows from a Mac

Needs cargo-xwin and LLVM:

```sh
rustup target add x86_64-pc-windows-msvc
cargo install cargo-xwin
brew install llvm makensis

PATH="/opt/homebrew/opt/llvm/bin:$PATH" \
  pnpm tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc --no-bundle
```

Use `--bundles nsis` instead of `--no-bundle` to build an installer.

## Reporting a bug

When you [open an issue](../../issues/new), include your OS, iPhone model, iOS
version and the error message, if any.
