<div align="center">

# Rasklad

**A tiny Windows tray tool that switches keyboard layout with a custom hotkey** — Caps Lock by default, so it stops being a dead key.

[![Release](https://img.shields.io/github/v/release/ProcessdNN/rasklad?logo=github)](https://github.com/ProcessdNN/rasklad/releases)
[![CI](https://github.com/ProcessdNN/rasklad/actions/workflows/ci.yml/badge.svg)](https://github.com/ProcessdNN/rasklad/actions/workflows/ci.yml)
[![Platform](https://img.shields.io/badge/platform-Windows-0078D6?logo=windows)]()
[![Rust](https://img.shields.io/badge/built%20with-Rust-orange?logo=rust)]()
[![License: GPL v3](https://img.shields.io/badge/license-GPL--3.0-green)](LICENSE)

</div>

## Download

Grab `rasklad.exe` from
[**Releases**](https://github.com/ProcessdNN/rasklad/releases) — a single
portable binary, no installer.

## Features

- **Caps Lock → next layout** out of the box; the key is swallowed so caps
  mode never turns on accidentally.
- **Custom hotkey**: record any key or combo — `Ctrl+Space`, `Alt+Q`,
  `F13`, or modifier-only combos like `Ctrl+Shift` — from the settings
  window.
- **Works everywhere**, including VM/RDP keyboards and injected input
  (AutoHotkey, kiosk setups): only rasklad's own synthetic keystrokes are
  filtered out.
- **Elevated windows**: injected input isn't blocked by UIPI, but a truly
  elevated foreground window still needs rasklad itself running as admin —
  the settings window detects that and offers a one-click elevated
  relaunch.
- **Tray-first**: lives in the notification area; left-click the icon for
  settings, right-click for the menu (Enabled / Quit).
- **Autostart your way**: registry Run key, Startup-folder shortcut, or a
  logon scheduled task — pick one in `⋯ → Autostart…`.
- **Advanced settings**: choose which combo toggles Caps Lock mode itself
  (`Shift+Caps` by default; `Ctrl`/`Alt`/`Win`+Caps, or Off).
- **Single instance**: launching the exe again just opens the settings
  window of the running copy.

## Usage

- First launch opens the settings window: enable/disable, record a new
  hotkey (press the combo — it's captured when you release it; `Esc`
  cancels), or open the `⋯` menu for Autostart / Advanced / Exit.
- Close the window to keep the app running in the tray; tray menu → Quit
  exits for real.
- Config lives in `%APPDATA%\rasklad\config.json`.

## Build

```powershell
cargo build --release
# target\release\rasklad.exe
```

Requires stable Rust with the `x86_64-pc-windows-msvc` toolchain
(`x86_64-pc-windows-gnu` also works).

## How it works

- A `WH_KEYBOARD_LL` low-level hook on a dedicated thread watches for the
  trigger key and eats it (returns without `CallNextHookEx`).
- `SendInput` injects the system's own layout-switch chord (`Alt+Shift`,
  `Ctrl+Shift` — whichever is bound in Windows, `Win+Space` as fallback),
  so the OS performs the switch exactly as if you pressed the Windows
  hotkey. Own keystrokes carry a `dwExtraInfo` tag so the hook can tell
  them apart from other injected input.
- `assets/icon.*` is embedded into the exe and used for the tray and
  window icons.

## License

GPL-3.0 (see [LICENSE](LICENSE)). Code committed before the v0.2.0
relicensing was published under the MIT license.
