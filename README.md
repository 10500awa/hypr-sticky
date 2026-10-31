# hyprsticky

Minimal sticky notes for the Hyprland window manager, written in Rust.

Every time you launch it you get a **new, empty note**. Each note is a markdown (`.md`) file in
`~/.local/share/hyprsticky/` (or `$XDG_DATA_HOME/hyprsticky/`), saved on every keystroke.
A note you never type in leaves no file behind. Built on egui/eframe with the Wayland +
OpenGL backends only: no GTK, no X11.

Because notes are plain markdown files, you can browse and organize them with Obsidian or
any other markdown tool (see [Obsidian](#obsidian)).

## Build

```sh
cargo install --path .        # installs to ~/.cargo/bin
# or: cargo build --release   # binary in target/release/hyprsticky
```

## Usage

```sh
hyprsticky              # a new, empty note
hyprsticky list         # saved notes, newest first (id, then first line)
hyprsticky "Buy milk"   # reopen a saved note by its name (the file name without .md)
hyprsticky todo         # open (or create) a note with a name you choose
```

`Esc` closes a note. Nothing is lost, since it saves as you type.
If you delete all the text in a note, its file is removed.

## Obsidian

Two ways to use your notes in Obsidian:

- Open the notes folder as a vault: in Obsidian choose "Open folder as vault" and pick
  `~/.local/share/hyprsticky/`.
- Or store notes inside a vault you already have by setting `HYPRSTICKY_DIR` to a folder in it
  (use an absolute path):

  ```sh
  export HYPRSTICKY_DIR="$HOME/Documents/MyVault/Stickies"
  ```

  For notes launched from Hyprland keybinds, set it in your Hyprland config's env section
  (`env = HYPRSTICKY_DIR,/home/you/Documents/MyVault/Stickies`) so every launch sees it.

While you type, a new note is saved under its creation time in milliseconds (e.g.
`1730372000000.md`). When you close it, it is renamed after its first line (e.g. `Buy milk.md`),
so Obsidian shows a readable title. If that name is already taken, a number is added (`Buy milk 2.md`).
Characters that are invalid in file names or break Obsidian links (`/ \ : * ? " < > | # ^ [ ]`)
are dropped, and long titles are cut to 60 characters.

Notes you named yourself (`hyprsticky todo`) keep their name. You can also rename notes freely
in Obsidian.

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
