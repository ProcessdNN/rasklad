# Rasklad

A tiny Windows tray tool that switches the keyboard layout with a custom
hotkey — **Caps Lock** by default — so it stops being a dead key.

> **Note:** early history (up to and including `v0.1.0`) was released
> under the MIT license; from `v0.2.0` onward Rasklad is licensed under
> **GPLv3**.

## Features

- **Caps Lock → next layout** out of the box; the key is suppressed so caps
  mode never turns on accidentally. `Shift+Caps` still toggles caps mode.
- **Custom hotkey**: record any key or combination (`Ctrl+Space`,
  `Alt+Q`, a single `F13`, ...) from the settings window.
- **Works everywhere**: the switch is performed by injecting the system's
  own layout-switch keystroke (`Alt+Shift`, `Ctrl+Shift` — whichever is
  bound in Windows, or `Win+Space` as fallback) via `SendInput`. For
  elevated (administrator) windows the app must run as admin — the
  settings window detects that and offers a one-click elevated
  relaunch.
- **Tray-first**: lives in the notification area; left-click the icon for
  settings, right-click for the menu (Enabled / Quit).
- **Autostart**: your pick — registry Run key, Startup-folder shortcut, or
  a logon scheduled task.
- Single instance: launching the exe again just opens the settings window
  of the running copy.

## Download

Grab `rasklad.exe` from
[Releases](https://github.com/ProcessdNN/rasklad/releases). It's a single
portable binary — no installer.

## Usage

- First launch opens the settings window: enable/disable, record a new
  hotkey, or open the ⋯ menu for autostart settings and Exit.
- Close the window to keep the app running in the tray; use tray menu →
  Quit to exit.
- Config is stored in `%APPDATA%\rasklad\config.json`.

## Build

```powershell
cargo build --release
# target\release\rasklad.exe
```

Requires stable Rust with the `x86_64-pc-windows-msvc` toolchain.

## How it works

- A `WH_KEYBOARD_LL` low-level hook on a dedicated thread watches for the
  trigger key and eats it (returning without calling `CallNextHookEx`).
- `SendInput` injects the layout-switch chord, so the OS performs the
  switch exactly as if you pressed the Windows hotkey.
- `assets/icon.*` is embedded into the exe and the tray/window icons.

## License

GPL-3.0 (see [LICENSE](LICENSE)). Code committed before the v0.2.0
relicensing was published under the MIT license.
