# t3agent37 on GPUI

The menu-bar app with its panel drawn natively by [GPUI](https://gpui.rs) (through [gpui-kit](https://gpui-kit.com) 0.10): no webview, no JavaScript. It runs the same backend as the Tauri app (`../src-tauri`, built without its `tauri-app` feature) on the same data folder, so it shows your real machines.

It is meant to replace the Tauri shell (`../src-tauri/src/tray.rs` and the React panel in `../src`).

## Run it

Quit the Tauri t3agent37 first. Both would run the same machines' proxies, ports and state. This app refuses to start while another t3agent37 holds the MCP port (47370).

```sh
cd apps/desktop/src-gpui
cargo run --release              # the menu-bar icon; click it
cargo run --release -- --open    # also opens the panel at launch
```

Needs macOS and Rust 1.99. `rust-toolchain.toml` picks 1.99 for this folder only, because gpui-pre 0.3.12 uses std APIs newer than 1.96.

- `T3AGENT37_DATA_DIR` points it at another data folder. The default is the Tauri app's, `~/Library/Application Support/dev.t3agent37.app/`.
- `T3AGENT37_KEEP_PANEL_OPEN=1` keeps the panel up when you click elsewhere (for recordings).
- `T3A_DEBUG=1` logs where the panel was placed.

Notifications and Start at login need an app bundle. From `cargo run`, notifications go through `osascript` (they show as Script Editor's), and Start at login says it needs the bundle.

## What's where

```
src/
  main.rs        startup, one instance at a time, the menu-bar item, showing and hiding the panel
  backend.rs     the backend on its own tokio runtime: setup, events, commands (as tray.rs)
  model.rs       what the panel does with a machine (as src/machine.ts)
  theme.rs       brand tokens and fonts (as src/brand.css)
  macos.rs       Rust side of macos/shell.m
  macos/shell.m  status item, panel placement, notifications, login item (after F2)
  ui/
    mod.rs       the panel: views, toasts, commands (as App.tsx)
    rooftop.rs   the strip (as Rooftop.tsx), compact in sub-views, scrolls when wide
    list.rs      machine rows, the ⋯ menu, costs (as MachineRow.tsx, Costs.tsx)
    details.rs   a machine's details (as MachineDetails.tsx)
    forms.rs     new machine and the project picker (as NewMachine.tsx, ProjectPicker.tsx)
    settings.rs  settings and first-run setup (as SettingsView.tsx, SetupView.tsx)
    kit.rs       buttons, pixel icons, switches, sections, agent logos
  pixel/         the rooftop's pixel engine, ported from src/pixel/*.ts
```

How it differs from the Tauri app:

- **The panel is a GPUI window** that is hidden, not closed, when it goes away. Everything it shows persists until the next click, like the webview did: a running sign-in, a half-typed name.
- **The panel calls the backend directly.** There's no JSON contract in between. `backend.rs` has the same startup and commands as `tray.rs`.
- **The wake link is copied by GPUI's clipboard**, not `pbcopy`.
- **Esc on the machine list closes the panel.** In sub-views it goes back, as before.

## Checking the rooftop port

`cargo test` checks the hash and seed functions against values computed by the TS code, and the panel's text helpers.

To compare whole pictures, render the TS engine's output with `scripts/ts-dump.ts`, then compare:

```sh
out=$(mktemp -d)
bun scripts/ts-dump.ts "$(cd ../src && pwd)" "$out"
T3A_TS_DUMP=$out cargo test matches_ts_renderer -- --nocapture
```

On 2026-11-20 that compared 86 pictures: rooftops with 1, 3, 5 and 9 agents in both themes at four times, and every state's row icon. All were byte-identical, tag positions included.

## Not done yet

- Bundling: an `.app` with its icon, like F2's `scripts/bundle-macos.sh`. Notifications and Start at login wait on it.
- Windows and Linux. The panel code is portable, but `shell.m` is macOS only.
- Scrollbars: views scroll, but no bar shows.
