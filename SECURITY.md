# Security

Rasklad sees every key you press while it runs. This page describes what it
does with that access, so you can decide whether that is acceptable.

## What rasklad does

- Installs a `WH_KEYBOARD_LL` hook while the app is running.
- Reads the virtual-key code of each press and whether Ctrl, Alt, Shift or Win
  are held. It does not build key sequences, does not record text and does not
  keep a history.
- Swallows the trigger key, and every other Caps Lock press while Caps Lock is
  the trigger.
- Switches the layout by sending `WM_INPUTLANGCHANGEREQUEST` to the foreground
  window, or by injecting `Win+Space` when that window runs elevated.
- Writes three things: `%APPDATA%\rasklad\config.json`, the autostart entry you
  selected, and, only when `RASKLAD_LOG` is set, a log file containing key
  codes.

## What rasklad does not do

- No network access. There is no HTTP client in the dependency tree and the
  binary opens no connections.
- No telemetry, no update check, no crash reporting.
- No stored input. A key press is examined and forgotten.

## Reporting a vulnerability

Use the **Report a vulnerability** button on the Security tab. It opens a
private advisory that only you and the maintainer can see.

Do not open a public issue for a security problem.

Please include the version, the Windows build, and what you did to trigger the
problem. A proof of concept helps more than a description.

## Supported versions

Only the latest release is supported. Fixes ship in a new release, and older
versions are not patched.
