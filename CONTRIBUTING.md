# Contributing to Rasklad

Rasklad is a small Windows tool maintained in spare time. Focused patches are
the easiest to review and the most likely to be merged.

## Building

Stable Rust with the `x86_64-pc-windows-msvc` toolchain. The GNU toolchain
works too.

```
cargo build --release
cargo test
```

The binary lands in `target\release\rasklad.exe`. There is no installer and
nothing to generate: the app is a single exe.

## Before opening a pull request

- `cargo test` passes.
- `cargo fmt` has been run.
- You tried the change on a real desktop. Most of this code talks to Win32,
  so the unit tests do not cover much of it.

A short manual pass covers the parts that break most often:

1. The hotkey switches the layout in Notepad and in a browser.
2. `Win+Caps` toggles Caps Lock mode, and Caps Lock itself stays off.
3. Recording a new hotkey works, and `Esc` cancels it.
4. Autostart survives a restart, whichever method is selected.
5. The tray menu opens and `Quit` exits.

## What fits the project

Rasklad does one thing: it switches the keyboard layout from a hotkey and
stays out of the way. It does not use the network, does not check for updates
and does not install anything.

New dependencies need a reason. Every crate ends up running with access to
everything the user types, so the bar here is higher than in a normal app.

## Commit messages

One change per commit. Subject in the imperative mood, around 70 characters,
no `feat:` or `fix:` prefixes. The body explains why the change is needed.

```
Switch layout via WM_INPUTLANGCHANGEREQUEST instead of injecting a hotkey

Injecting the system's own switch hotkey does nothing when that hotkey is
unbound, which is the default on many Windows 11 installs.
```

## License

Rasklad is licensed under GPL-3.0. By contributing you agree that your
changes are published under the same license.
