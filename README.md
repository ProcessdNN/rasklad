# capswitch

A tiny Windows tray tool that switches the keyboard layout with a custom
hotkey — **Caps Lock** by default — so it stops being a dead key.

## Features

- **Caps Lock → next layout** out of the box; the key is suppressed so caps
  mode never turns on accidentally. `Shift+Caps` still toggles caps mode.
- **Custom hotkey**: record any key or combination (`Ctrl+Space`,
  `Alt+Q`, a single `F13`, ...) from the settings window.
- **Works everywhere**: the switch is performed by injecting the system's
  own layout-switch keystroke (`Alt+Shift`, `Ctrl+Shift` — whichever is
  bound in Windows, or `Win+Space` as fallback) via `SendInput`. Injected
  input isn't filtered by UIPI, so it also works while an elevated
  (administrator) window is focused — no admin rights needed.
- **Tray-first**: lives in the notification area; left-click the icon for
  settings, right-click for the menu (Settings / Enabled / Quit).
- **Autostart**: one checkbox registers it in `HKCU\...\Run` (starts
  minimized to tray).
- Single instance: launching the exe again just opens the settings window
  of the running copy.

## Download

Grab `capswitch.exe` from
[Releases](https://github.com/ProcessdNN/capswitch/releases). It's a single
portable binary — no installer.

## Usage

- First launch opens the settings window: enable/disable, record a new
  hotkey, toggle autostart.
- Close the window to keep the app running in the tray; use tray menu →
  Quit to exit.
- Config is stored in `%APPDATA%\capswitch\config.json`.

## Build

```powershell
cargo build --release
# target\release\capswitch.exe
```

Requires stable Rust with the `x86_64-pc-windows-msvc` toolchain.

## How it works

- A `WH_KEYBOARD_LL` low-level hook on a dedicated thread watches for the
  trigger key and eats it (returning without calling `CallNextHookEx`).
- `SendInput` injects the layout-switch chord, so the OS performs the
  switch exactly as if you pressed the Windows hotkey.
- `assets/icon.*` is embedded into the exe and the tray/window icons.

## License

MIT
