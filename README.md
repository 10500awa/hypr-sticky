# hypr-sticky

Minimal sticky notes for the Hyprland window manager, written in Rust.

One window = one note. Notes are plain text files in
`~/.local/share/hypr-sticky/<name>.txt` (or `$XDG_DATA_HOME`), saved on every keystroke.
Built on egui/eframe with the Wayland + OpenGL backends only: no GTK, no X11.

## Build

```sh
cargo build --release
install -Dm755 target/release/hypr-sticky ~/.local/bin/hypr-sticky
```

## Usage

```sh
hypr-sticky          # opens the note called "note"
hypr-sticky todo     # opens (or creates) the note called "todo"
```

`Ctrl+W` closes a note. Nothing is lost, since it saves as you type.

## Hyprland config

Notes are normal windows, so let Hyprland float and pin them.

Hyprland 0.53 and newer:

```ini
hl.window_rule({
  name = "hypr-sticky",
  float = true,
  pin = true,
  size = "260 260",
  match = {
    class = "hypr-sticky"
  }
})

```
Keybinds and autostart (notes have no title bar, so move and resize them with the mouse):

```ini

hl.bind(mod .. " + SHIFT + N", hl.dsp.exec_cmd("hypr-sticky"))
hl.bind(mod .. " + SHIFT + T", hl.dsp.exec_cmd("hypr-sticky todo"))
hl.bindm(mod + " + mouse:272", hl.dsp.window.move())
hl.bindm(mod + " + mouse:273", hl.dsp.window.resize())
hl.exec_once("hypr-sticky todo")
```

## License

See [LICENSE](LICENSE).
