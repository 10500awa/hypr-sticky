# hyprsticky

Minimal sticky notes for the Hyprland window manager, written in Rust.

Every time you launch it you get a **new, empty note**. Each note is a plain text file in
`~/.local/share/hyprsticky/` (or `$XDG_DATA_HOME/hyprsticky/`), saved on every keystroke.
A note you never type in leaves no file behind. Built on egui/eframe with the Wayland +
OpenGL backends only: no GTK, no X11.

## Build

```sh
cargo install --path .        # installs to ~/.cargo/bin
# or: cargo build --release   # binary in target/release/hyprsticky
```

## Usage

```sh
hyprsticky              # a new, empty note
hyprsticky list         # saved notes, newest first (id, then first line)
hyprsticky 1730372000000   # reopen a saved note by its id
hyprsticky todo         # open (or create) a note with a name you choose
```

`Ctrl+W` closes a note. Nothing is lost, since it saves as you type.
If you delete all the text in a note, its file is removed.

## Hyprland config

Notes are normal windows, so let Hyprland float and pin them. The window class is `hyprsticky`.

### Lua config

```lua
hl.window_rule({
  name = "hyprsticky",
  float = true,
  pin = true,
  size = "260 260",
  match = {
    class = "hyprsticky"
  }
})
```

Keybinds and autostart (notes have no title bar, so move and resize them with the mouse):

```lua
hl.bind(mod .. " + SHIFT + N", hl.dsp.exec_cmd("hyprsticky"))
hl.bindm(mod .. " + mouse:272", hl.dsp.window.move())
hl.bindm(mod .. " + mouse:273", hl.dsp.window.resize())
hl.exec_once("hyprsticky todo")
```

### hyprlang config (Hyprland 0.53 and newer)

```ini
windowrule = float on, match:class ^(hyprsticky)$
windowrule = pin on,   match:class ^(hyprsticky)$
windowrule = size 260 260, match:class ^(hyprsticky)$

bind   = SUPER SHIFT, N, exec, hyprsticky
bindm  = SUPER, mouse:272, movewindow
bindm  = SUPER, mouse:273, resizewindow
```

### Older versions

```ini
windowrulev2 = float, class:^(hyprsticky)$
windowrulev2 = pin,   class:^(hyprsticky)$
windowrulev2 = size 260 260, class:^(hyprsticky)$
```

Keybinds and `exec-once` don't always see your shell's `PATH`; if `hyprsticky` isn't found,
use the full path, e.g. `~/.cargo/bin/hyprsticky`.

## License

See [LICENSE](LICENSE).
