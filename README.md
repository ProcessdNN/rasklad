<div align="center">

<img src="assets/icon.png" width="112" alt="Rasklad icon">

# Rasklad

[RU](README.ru.md) / **EN**

A small tray tool for Windows that switches the keyboard layout with a hotkey.

[![Release](https://img.shields.io/github/v/release/ProcessdNN/rasklad?logo=github)](https://github.com/ProcessdNN/rasklad/releases)
[![CI](https://github.com/ProcessdNN/rasklad/actions/workflows/ci.yml/badge.svg)](https://github.com/ProcessdNN/rasklad/actions/workflows/ci.yml)
[![Platform](https://img.shields.io/badge/platform-Windows-0078D6?logo=windows)]()
[![Rust](https://img.shields.io/badge/built%20with-Rust-orange?logo=rust)]()
[![License: GPL v3](https://img.shields.io/badge/license-GPL--3.0-green)](LICENSE)

</div>

## Overview

Caps Lock is a dead key on most keyboards. Rasklad turns it into a layout
switch. Press it, the layout changes, and the key itself never reaches the
application, so caps mode does not turn on by accident.

The app lives in the notification area and has no window unless you open the
settings. It is one exe with no installer, no runtime and no network access.

## Features

- **Any hotkey.** Record a combination like `Ctrl+Space` or `Alt+Q`, or a key
  that works on its own such as `F13` or `PrintScreen`. A bare modifier, or a
  letter with no modifier, is refused, and the window says why.
- **Switching that does not depend on the system hotkey.** Rasklad asks the
  foreground window to activate the next layout instead of pressing
  `Alt+Shift` for you. It works on installs where that chord is unbound and
  only `Win+Space` is left, and no language overlay appears.
- **Caps Lock mode on your terms.** `Shift+Caps` by default, or `Ctrl`, `Alt`,
  `Win`, or nothing at all.
- **Autostart, checked.** Registry, Startup folder or a scheduled task. On
  every start the entry is read back: a dead one is recreated, one that points
  at another build is reported in the window.
- **Tray first.** Left click opens the settings, right click opens the menu.
  Closing the window keeps the app running.
- **One instance.** Launching the exe again opens the window of the running
  copy.
- **No network.** No telemetry, no update check, no HTTP client in the
  dependency tree.

## Install

Download `rasklad.exe` from
[**Releases**](https://github.com/ProcessdNN/rasklad/releases) and run it.
Nothing else is needed: no installer, no runtime, no redistributable.

Windows 10 and 11, 64-bit.

Windows asks for administrator rights when the exe starts. The hook needs them
to switch layouts in elevated windows, and there is no second mode without
them.

The first launch opens the settings window. Turn on autostart there if you want
the app back after a reboot.

## Usage

- Press the hotkey in any application. The layout switches, and the key does
  not reach the application.
- Press `Win+Caps` to toggle caps mode, or whichever combination is set in
  Advanced.
- Left click the tray icon for settings, right click for the menu.

## Settings

| Control | What it does |
|---|---|
| Enabled | Master switch for the hotkey. |
| Hotkey, Record | Press the combination. `Esc` cancels. |
| `⋯` → Autostart | Registry Run key, Startup folder shortcut or a logon task. |
| `⋯` → Advanced | Which combination toggles caps mode. |
| `⋯` → Exit | Quit. Closing the window only hides it. |

The config file is `%APPDATA%\rasklad\config.json`.

## Build

```
cargo build --release
```

Requires stable Rust with the `x86_64-pc-windows-msvc` toolchain
(`x86_64-pc-windows-gnu` also works). The exe lands in `target\release`.

## How it works

- A `WH_KEYBOARD_LL` hook on a dedicated thread watches key presses and eats
  the trigger.
- The layout is switched with `WM_INPUTLANGCHANGEREQUEST` sent to the
  foreground window. Because rasklad runs elevated itself, that message is
  accepted by elevated windows too; `Win+Space` is only injected when the
  message is refused.
- Modifier state is read from `GetAsyncKeyState` on every event. Nothing is
  accumulated, so a missed key release cannot leave the hotkey stuck.
- Autostart entries are read back at startup, which is how a stale registry
  value or shortcut gets noticed.

## Known limitations

- The hotkey does not fire while the rasklad window itself is focused. Windows
  does not deliver those keystrokes to the hook. Recording a hotkey is not
  affected, because it reads the key state directly.

## Contributing

Patches are welcome, see [CONTRIBUTING.md](CONTRIBUTING.md). Questions and
reports go through [SUPPORT.md](SUPPORT.md), security issues through
[SECURITY.md](SECURITY.md).

## License

GPL-3.0, see [LICENSE](LICENSE). Code committed before the v0.2.0 relicensing
was published under the MIT license.
