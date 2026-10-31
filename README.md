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
windowrule = float on, match:class ^(hypr-sticky)$
windowrule = pin on,   match:class ^(hypr-sticky)$
windowrule = size 260 260, match:class ^(hypr-sticky)$
```

Older versions:

```ini
windowrulev2 = float, class:^(hypr-sticky)$
windowrulev2 = pin,   class:^(hypr-sticky)$
windowrulev2 = size 260 260, class:^(hypr-sticky)$
```

Keybinds and autostart (notes have no title bar, so move and resize them with the mouse):

```ini
bind   = SUPER SHIFT, N, exec, hypr-sticky
bind   = SUPER SHIFT, T, exec, hypr-sticky todo
bindm  = SUPER, mouse:272, movewindow
bindm  = SUPER, mouse:273, resizewindow
exec-once = hypr-sticky todo
```

## License

See [LICENSE](LICENSE).
