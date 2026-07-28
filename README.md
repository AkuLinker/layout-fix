# layout-fix

Hotkey utility for KDE Wayland that fixes text typed in the wrong keyboard
layout (EN/RU) — no background daemon, no keylogging. Select the garbled
text, press a hotkey, paste the corrected version.

> **⚠️ Only supports the EN ↔ RU layout pair.** Any other
> layout combination will produce garbage.
>
> **⚠️ Работает только с парой раскладок EN ↔ RU.** С любой
> другой раскладкой результат будет нечитаемой кашей.

## How it works

1. You select some text (mouse or Shift+arrows) — no need to copy it.
2. You press a hotkey.
3. The tool reads the selection, detects whether it's more Latin or
   Cyrillic letters, and converts it character-by-character using a table
   derived directly from the `us` / `ru` XKB layouts.
4. The result goes into the clipboard, and a native KDE OSD (the same
   widget used for layout-switch/volume popups) briefly shows a preview.
5. You paste it yourself with Ctrl+V, replacing the still-selected text.

The binary is not resident — it runs once per hotkey press and exits.
Nothing is monitored in the background.

## Requirements

- KDE Plasma on Wayland (uses `org.kde.plasmashell`'s OSD service over
  D-Bus)
- [`wl-clipboard`](https://github.com/bugaevc/wl-clipboard) (`wl-paste` /
  `wl-copy`), for example: `sudo pacman -S wl-clipboard`

## Install

Build from source:

```bash
git clone https://github.com/<you>/layout-fix.git
cd layout-fix
cargo build --release
```

Then:

```bash
mkdir -p ~/.local/bin
cp target/release/layout-fix ~/.local/bin/
```

Or grab a prebuilt binary from [Releases](../../releases).

## Set up the hotkey

System Settings → Keyboard → Shortcuts → Add New → Command or Script.

- **Command/URL:** `/home/<you>/.local/bin/layout-fix`
- **Trigger:** any unused combination, e.g. `Meta+;`

## Character mapping

Conversion direction (EN→RU or RU→EN) is decided once per selection, based
on whichever script (Latin or Cyrillic) has more letters in the text. If
the selection has no letters at all (only digits/punctuation), the tool
shows a notice instead of guessing.

The character table itself is a straight physical-key mapping between the
`us` and `ru(winkeys)` XKB layouts — not a heuristic. Letters convert
fully; punctuation that shares a key with a Cyrillic letter (`, . ; ' [ ]
\`` etc.) converts too; symbols identical on both layouts (`% * ( ) - _ =
+`, digits) are left untouched, since they simply aren't in the table.
Anything outside both layouts — other scripts, emoji, diacritics — passes
through unchanged.

## Known limitations

- KDE's Klipper keeps the last PRIMARY selection indefinitely. If nothing
  is currently selected but something was selected earlier in the
  session, the tool will act on that old value instead of doing nothing.
- Mixed-language selections (e.g. "hello мир") are converted based on
  majority script, which may not match what you expect.
