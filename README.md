# hyprsticky

Tiny sticky notes for Hyprland, written in Rust.

It uses about **4.4 MB** of RAM (1.8 MB of it private, measured on the author's machine). An
earlier egui version of the same app used about 168 MB. There is no GPU, GTK or toolkit: it draws
on the CPU and talks to Wayland directly.

- Every launch opens a new, empty note.
- Notes are plain markdown files in `~/.local/share/hyprsticky/`, saved automatically.
- Works with input methods (fcitx, ibus) and shows Chinese, Japanese and Korean text.

## Install

```sh
cargo install --path .
```

## Usage

```sh
hyprsticky              # new note
hyprsticky list         # saved notes
hyprsticky "Buy milk"   # reopen a note by name
```

`Esc` closes a note. Notes save a moment after you stop typing, and are renamed after their
first line when closed.

Keys: arrows, Home/End, Ctrl+arrows (by word), Ctrl+A, Ctrl+Z, Ctrl+Shift+Z.
Mouse: click, drag to select, double-click for a word, triple-click for a line.

## Hyprland

Lua config:

```lua
hl.window_rule({ name = "hyprsticky", float = true, pin = true, size = "260 260",
                 match = { class = "hyprsticky" } })
hl.bind(mod .. " + SHIFT + N", hl.dsp.exec_cmd("hyprsticky"))
```

hyprlang config:

```ini
windowrule = float on, match:class ^(hyprsticky)$
windowrule = pin on, match:class ^(hyprsticky)$
windowrule = size 260 260, match:class ^(hyprsticky)$
bind = SUPER SHIFT, N, exec, hyprsticky
```

Notes have no title bar, so move them with `SUPER` + drag. If a keybind can't find
`hyprsticky`, use the full path (`~/.cargo/bin/hyprsticky`).

## Settings

- `HYPRSTICKY_DIR`: where notes are stored. Point it at a folder in an Obsidian vault to manage
  them there.
- `HYPRSTICKY_FONT`: a fontconfig pattern (`monospace`) or a font file. Default: `sans-serif`.

## Limitations

No copy and paste yet, and markdown is not rendered.

## License

See [LICENSE](LICENSE).
