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
- **Custom hotkey**: record any combo — `Ctrl+Space`, `Alt+Q`, `F13` — or
  a key that stands on its own (`F1`–`F24`, `Home`/`End`/`PgUp`/`PgDn`,
  arrows, `PrintScreen`, `Caps`). A bare modifier, or a plain letter with
  no modifier, is refused — with the reason shown right in the window.
- **Works everywhere**, including VM/RDP keyboards and injected input
  (AutoHotkey, kiosk setups): only rasklad's own synthetic keystrokes are
  filtered out.
- **Elevated windows**: injected input isn't blocked by UIPI, but a truly
  elevated foreground window still needs rasklad itself running as admin —
  the settings window detects that and offers a one-click elevated
  relaunch.
- **Tray-first**: lives in the notification area; left-click the icon for
  settings, right-click for the menu (Enabled / Quit).
- **Switching that does not depend on Windows' own hotkey**: rasklad asks
  the foreground window to activate the next layout instead of pressing
  `Alt+Shift` for you, so it works on installs where that chord is unbound
  (only `Win+Space` left) — and no language overlay flashes up.
- **Autostart your way**: registry Run key, Startup-folder shortcut, or a
  logon scheduled task — pick one in `⋯ → Autostart…`. The entry is
  checked on every start: one that points at a deleted exe (or has an
  empty command left by an older version) is recreated, and one that
  points at a different build is reported in the Autostart window.
- **Advanced settings**: choose which combo toggles Caps Lock mode itself
  (`Shift+Caps` by default; `Ctrl`/`Alt`/`Win`+Caps, or Off).
- **Single instance**: launching the exe again just opens the settings
  window of the running copy.

## Usage

- First launch opens the settings window: enable/disable, record a new
  hotkey (press the combo — the first real key you press becomes the
  hotkey, `Esc` cancels), or open the `⋯` menu for Autostart / Advanced /
  Exit.
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
- The layout is switched by asking the foreground window to activate the
  next layout (`WM_INPUTLANGCHANGEREQUEST`) — no synthetic keys, no
  overlay. If the window cannot be asked (it runs elevated and UIPI blocks
  the message), `Win+Space` is injected instead; those keystrokes carry a
  `dwExtraInfo` tag so the hook tells them apart from other injected input
  (AutoHotkey, VMs, kiosks).
- Modifier state is read from `GetAsyncKeyState` on each key event instead
  of being accumulated, so a missed key-up cannot leave the hotkey stuck.
- `assets/icon.*` is embedded into the exe and used for the tray and
  window icons.

## Known limitations

- The hotkey does not fire while rasklad's own settings window is focused —
  Windows does not deliver those keystrokes to the hook. Recording a
  hotkey is unaffected, because it reads the physical key state instead.
- Layout switching in a foreground window that runs elevated still needs
  rasklad itself to run as admin.

## License

GPL-3.0 (see [LICENSE](LICENSE)). Code committed before the v0.2.0
relicensing was published under the MIT license.
