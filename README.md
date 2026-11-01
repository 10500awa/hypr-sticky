# hyprsticky

Tiny sticky notes for Hyprland, written in Rust. (vibecoded of course)

It uses about **4 MB** of ram.

- Every launch opens a new, empty note.
- Notes are plain markdown files in `~/.local/share/hyprsticky/`, saved automatically.
- Works with input methods (fcitx, ibus) and shows Chinese, Japanese and Korean text.
- Markdown is styled as you type: headings, bold, italic, strikethrough, code, links, quotes,
  lists, task lists and tables (monospace). The markers stay visible, dimmed.

## Install

```sh
git clone https://github.com/10500awa/hyprsticky
cd hyprsticky
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

Bind:
```lua
hl.bind(mod .. " + SHIFT + N", hl.dsp.exec_cmd("hyprsticky"))
```
Window rule:
```lua
hl.window_rule({
name = "hyprsticky",
float = true,
pin = true,
size = "260 260",
match = { class = "hyprsticky" }
})
``` 

Notes have no title bar, so move them with `SUPER` + drag. If a keybind can't find
`hyprsticky`, use the full path (`~/.cargo/bin/hyprsticky`).

## Settings

- `HYPRSTICKY_DIR`: where notes are stored. Point it at a folder in an Obsidian vault to manage
  them there.
- `HYPRSTICKY_FONT`: a fontconfig pattern (`monospace`) or a font file. Default: `sans-serif`.

## Limitations

No copy and paste yet. Images and links are shown as styled text, not opened or displayed.

## License

See [LICENSE](LICENSE).
