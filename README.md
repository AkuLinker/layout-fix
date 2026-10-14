# layout-fix

Hotkey utility for KDE Wayland that fixes text typed in the wrong keyboard
layout — no background daemon, no keylogging. Select the garbled text,
press a hotkey, paste the corrected version (or let the tool paste it for you).

Works with whatever layouts you have configured in KDE (EN ↔ RU, EN ↔ UA,
RU (winkeys) ↔ US, …): the character mapping is read from XKB at runtime,
not hard-coded.

## How it works

1. You select some text (mouse or Shift+arrows) — no need to copy it.
2. You press a hotkey.
3. The tool reads the selection and, word by word, finds out which of your
   layouts it was typed in, then converts it to the same physical keys in
   the other layout.
4. The result goes into the clipboard, and a native KDE OSD (the same
   widget used for layout-switch/volume popups) briefly shows a preview.
5. You paste it with Ctrl+V, replacing the still-selected text — or, with
   `--paste`, the tool does it and then puts your old clipboard back.

The binary is not resident — it runs once per hotkey press and exits.
Nothing is monitored in the background.

The converted text is marked as sensitive (`x-kde-passwordManagerHint`),
so Klipper and other clipboard managers that respect the hint don't add it
to their history.

## Requirements

- KDE Plasma on Wayland (uses the Plasma OSD and the KDE keyboard layout
  service over D-Bus)
- [`wl-clipboard`](https://github.com/bugaevc/wl-clipboard) (`wl-paste` /
  `wl-copy`), for example: `sudo pacman -S wl-clipboard`. Version 2.3 or
  newer is needed to keep the converted text out of clipboard history;
  older versions still work, just without that.
- `libxkbcommon` and `xkeyboard-config` (already installed with Plasma)
- Optional, for `--paste`: [`ydotool`](https://github.com/ReimuNotMoe/ydotool)
  with the `ydotoold` service running, e.g. on Arch:
  `sudo pacman -S ydotool && systemctl --user enable --now ydotool`.
  ydotoold needs access to `/dev/uinput`; see your distribution's ydotool
  documentation if it can't open it.

## Install

Build from source:

```bash
git clone https://github.com/AkuLinker/layout-fix.git
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

- **Command/URL:** `/home/<you>/.local/bin/layout-fix` plus any options
  below, e.g. `/home/<you>/.local/bin/layout-fix --paste --switch-layout`
- **Trigger:** any unused combination, e.g. `Meta+;`

You can add several shortcuts with different options.

## Options

| Option | What it does |
| --- | --- |
| *(none)* | Convert and put the result in the clipboard; you press Ctrl+V |
| `--paste` | Paste automatically, then restore what was in the clipboard before |
| `--paste-delay <MS>` | Wait this long before pasting (default 150), so you have time to release the hotkey's modifier keys |
| `--switch-layout` | Switch the active keyboard layout to the one the text was converted into |

If nothing gets pasted, or some other shortcut fires instead, your hotkey's
modifier keys were probably still held down: increase `--paste-delay`.

## Character mapping

Each layout you've configured (read from `~/.config/kxkbrc`, including
variants) is compiled with libxkbcommon, and the tool records which
character each key of the main typing block produces, with and without
Shift. A character is converted by finding its key in the source layout
and taking what that key gives in the target layout. Characters that are
the same in both layouts (digits, `% * ( ) - _ = +`, …) stay the same, and
anything the source layout can't type — other scripts, emoji, diacritics —
passes through unchanged.

Direction is decided per word: each word is converted from the layout that
can type most of its letters. Words without letters (numbers, lone
punctuation) follow the nearest word that has letters. With more than two
layouts, the target is the currently active layout if it differs from the
source (you've already switched to the one you meant), otherwise the next
layout in your list. If the selection has no letters at all, the tool shows
a notice instead of guessing.

## Known limitations

- KDE's Klipper keeps the last PRIMARY selection indefinitely. If nothing
  is currently selected but something was selected earlier in the
  session, the tool will act on that old value instead of doing nothing.
- With `--paste`, only one format of the previous clipboard is restored:
  plain text for text, or the original type (e.g. `image/png`) for other
  data. Rich formats like HTML are dropped.
- With `--paste`, restoring happens 300 ms after pasting. An unusually slow
  app could paste the restored content instead; this is rare.
