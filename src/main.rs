//! hyprsticky: minimal sticky notes for Hyprland.
//!
//! A plain Wayland client that draws into shared memory with the CPU: no GPU, no toolkit.
//! That keeps memory use small: about 4.4 MB resident (1.8 MB of it private) with a note open,
//! measured on one machine, against roughly 168 MB for the earlier egui/OpenGL version.
//! Each note is one window and one markdown file.

mod font;
mod layout;
mod store;
mod text;

use std::time::{Duration, Instant};

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm, delegate_xdg_shell, delegate_xdg_window,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::EventLoop,
        calloop_wayland_source::WaylandSource,
        client::{
            Connection, Dispatch, QueueHandle,
            globals::registry_queue_init,
            protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
        },
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RepeatInfo},
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::{
        WaylandSurface,
        xdg::{
            XdgShell,
            window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
        },
    },
    shm::{
        Shm, ShmHandler,
        slot::{Buffer, SlotPool},
    },
};

use font::{Bitmap, Fonts};
use layout::Layout;
use store::APP_ID;
use text::Editor;
use wayland_protocols::wp::text_input::zv3::client::{
    zwp_text_input_manager_v3, zwp_text_input_v3,
};

const DEFAULT_SIZE: u32 = 260;
/// Font size in logical pixels per em.
const FONT_PX: f32 = 15.0;
/// Space between the window edge and the text, in logical pixels.
const MARGIN: f32 = 10.0;

const PAPER: [u8; 3] = [255, 241, 118];
const INK: [u8; 3] = [45, 40, 20];
const SELECTION: [u8; 3] = [240, 196, 48];

/// Save this long after the last edit...
const SAVE_PAUSE: Duration = Duration::from_secs(2);
/// ...but never wait longer than this while typing continuously.
const SAVE_MAX: Duration = Duration::from_secs(10);

const BTN_LEFT: u32 = 0x110;
const DOUBLE_CLICK_MS: u32 = 400;

/// An XRGB/ARGB8888 (little endian: B, G, R, A) pixel buffer.
struct Canvas<'a> {
    data: &'a mut [u8],
    w: i32,
    h: i32,
}

impl Canvas<'_> {
    fn fill(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: [u8; 3]) {
        let (x0, x1) = (x0.clamp(0, self.w), x1.clamp(0, self.w));
        let (y0, y1) = (y0.clamp(0, self.h), y1.clamp(0, self.h));
        for y in y0..y1 {
            for x in x0..x1 {
                let o = ((y * self.w + x) * 4) as usize;
                self.data[o] = c[2];
                self.data[o + 1] = c[1];
                self.data[o + 2] = c[0];
                self.data[o + 3] = 255;
            }
        }
    }

    /// Blends a glyph's coverage mask over the existing pixels.
    fn blend_glyph(&mut self, g: &Bitmap, x: i32, y: i32, c: [u8; 3]) {
        for gy in 0..g.h as i32 {
            let py = y + gy;
            if py < 0 || py >= self.h {
                continue;
            }
            for gx in 0..g.w as i32 {
                let px = x + gx;
                if px < 0 || px >= self.w {
                    continue;
                }
                let cov = g.cov[(gy * g.w as i32 + gx) as usize] as u32;
                if cov == 0 {
                    continue;
                }
                let o = ((py * self.w + px) * 4) as usize;
                for k in 0..3 {
                    let dst = self.data[o + k] as u32;
                    let src = c[2 - k] as u32;
                    self.data[o + k] = ((dst * (255 - cov) + src * cov + 127) / 255) as u8;
                }
            }
        }
    }
}

/// Draws the whole note. `s` is the buffer scale; `scroll_y` is in buffer pixels.
fn render(
    cv: &mut Canvas,
    s: f32,
    editor: &Editor,
    layout: &Layout,
    fonts: &mut Fonts,
    scroll_y: f32,
) {
    cv.fill(0, 0, cv.w, cv.h, PAPER);
    let chars = editor.chars();
    if layout.text_end() != chars.len() {
        return; // layout is stale; it is rebuilt before the next draw
    }

    let margin = MARGIN * s;
    let lh = fonts.line_height;
    let baseline = fonts.baseline;
    let space_w = fonts.space_width;
    let first = (scroll_y / lh).floor().max(0.0) as usize;
    let last = (((scroll_y + cv.h as f32) / lh).ceil() as usize + 1).min(layout.lines.len());
    let selection = editor.selection();

    for l in first..last {
        let line = &layout.lines[l];
        let top = margin + l as f32 * lh - scroll_y;
        let (y0, y1) = (top.round() as i32, (top + lh).round() as i32);

        if let Some((a, b)) = selection {
            if a <= line.end && b >= line.start {
                let lo = a.clamp(line.start, line.end);
                let hi = b.clamp(line.start, line.end);
                let x0 = margin + line.xs[lo - line.start];
                let x1 = margin + line.xs[hi - line.start];
                if x1 > x0 {
                    cv.fill(x0.round() as i32, y0, x1.round() as i32, y1, SELECTION);
                }
                // A selected line break is shown as a small extra block at the end of the line.
                if line.end < chars.len() && !layout.is_wrapped(l) && a <= line.end && b > line.end
                {
                    let xe = margin + line.xs[line.end - line.start];
                    cv.fill(
                        xe.round() as i32,
                        y0,
                        (xe + space_w).round() as i32,
                        y1,
                        SELECTION,
                    );
                }
            }
        }

        let base_y = (top + baseline).round() as i32;
        for (col, &c) in chars[line.start..line.end].iter().enumerate() {
            if let Some(g) = fonts.glyph(c) {
                let x = (margin + line.xs[col]).round() as i32 + g.left;
                cv.blend_glyph(g, x, base_y + g.top, INK);
            }
        }
    }

    // Caret
    let (l, col) = layout.locate(editor.cursor());
    let x = (margin + layout.lines[l].xs[col]).round() as i32;
    let top = margin + l as f32 * lh - scroll_y;
    let width = (s.round() as i32).max(1);
    cv.fill(
        x,
        top.round() as i32,
        x + width,
        (top + lh).round() as i32,
        INK,
    );
}

struct Repeat {
    event: KeyEvent,
    next: Instant,
}

struct App {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    window: Window,
    pool: SlotPool,
    buffer: Option<Buffer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,

    // Input method (IME) support, through the text-input protocol.
    text_manager: Option<zwp_text_input_manager_v3::ZwpTextInputManagerV3>,
    text_input: Option<zwp_text_input_v3::ZwpTextInputV3>,
    ime_active: bool,
    /// Characters of the in-progress composition, which sit just before the cursor.
    composing: usize,
    pending_commit: Option<String>,
    pending_preedit: String,

    // Window geometry in logical pixels, and the integer buffer scale.
    width: u32,
    height: u32,
    scale: i32,
    configured: bool,
    needs_draw: bool,
    exit: bool,
    finished: bool,

    // The note.
    path: std::path::PathBuf,
    editor: Editor,
    layout: Layout,
    fonts: Fonts,
    scroll_y: f32,
    goal_x: Option<f32>,
    dirty_since: Option<Instant>,
    last_edit: Instant,

    // Input.
    mods: Modifiers,
    repeat: Option<Repeat>,
    repeat_delay: Duration,
    repeat_interval: Option<Duration>,
    mouse: (f64, f64),
    dragging: bool,
    click_count: u32,
    last_click: Option<(u32, (f64, f64))>,
}

impl App {
    // ---- layout and scrolling ----

    fn relayout(&mut self) {
        let s = self.scale.max(1) as f32;
        self.fonts.set_px(FONT_PX * s);
        let max_w = (self.width as f32 * s - 2.0 * MARGIN * s).max(1.0);
        let fonts = &mut self.fonts;
        self.layout = Layout::build(self.editor.chars(), max_w, &mut |c| fonts.advance(c));
    }

    fn view_height(&self) -> f32 {
        let s = self.scale.max(1) as f32;
        (self.height as f32 * s - 2.0 * MARGIN * s).max(1.0)
    }

    fn clamp_scroll(&mut self) {
        let content = self.layout.lines.len() as f32 * self.fonts.line_height;
        let max = (content - self.view_height()).max(0.0);
        self.scroll_y = self.scroll_y.clamp(0.0, max);
    }

    fn ensure_cursor_visible(&mut self) {
        let lh = self.fonts.line_height;
        let (l, _) = self.layout.locate(self.editor.cursor());
        let top = l as f32 * lh;
        let bottom = top + lh;
        let view = self.view_height();
        if top < self.scroll_y {
            self.scroll_y = top;
        } else if bottom > self.scroll_y + view {
            self.scroll_y = bottom - view;
        }
        self.clamp_scroll();
    }

    fn set_scale(&mut self, scale: i32) {
        if scale >= 1 && scale != self.scale {
            self.scale = scale;
            self.buffer = None;
            self.relayout();
            self.clamp_scroll();
            self.needs_draw = true;
        }
    }

    // ---- saving ----

    fn save_now(&mut self) {
        store::write_note(&self.path, &self.editor.text());
        self.dirty_since = None;
    }

    fn save_deadline(&self) -> Option<Instant> {
        self.dirty_since
            .map(|since| (self.last_edit + SAVE_PAUSE).min(since + SAVE_MAX))
    }

    /// Saves and names the note after its first line. Safe to call more than once.
    fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        // Text the input method has not committed yet is dropped, not saved.
        self.clear_composition();
        if self.dirty_since.is_some() {
            self.save_now();
        }
        if let Some(p) = store::rename_to_title(&self.path, &self.editor.text()) {
            self.path = p;
        }
    }

    fn close(&mut self) {
        self.finish();
        self.exit = true;
    }

    // ---- timers ----

    fn next_timeout(&self) -> Option<Duration> {
        let now = Instant::now();
        let mut soonest: Option<Duration> = None;
        let mut consider = |at: Instant| {
            let d = at.saturating_duration_since(now);
            soonest = Some(soonest.map_or(d, |s| s.min(d)));
        };
        if let (Some(r), Some(_)) = (&self.repeat, self.repeat_interval) {
            consider(r.next);
        }
        if let Some(at) = self.save_deadline() {
            consider(at);
        }
        soonest
    }

    fn tick(&mut self) {
        let now = Instant::now();
        let due = match (&mut self.repeat, self.repeat_interval) {
            (Some(r), Some(interval)) if now >= r.next => {
                r.next = now + interval;
                Some(r.event.clone())
            }
            _ => None,
        };
        if let Some(event) = due {
            self.handle_key(&event);
        }
        if self.save_deadline().is_some_and(|d| now >= d) {
            self.save_now();
        }
    }

    // ---- input method ----

    /// Removes the in-progress composition from the note.
    fn clear_composition(&mut self) {
        if self.composing == 0 {
            return;
        }
        let end = self.editor.cursor();
        self.editor
            .select_range(end.saturating_sub(self.composing), end);
        self.editor.backspace();
        self.composing = 0;
        self.dirty_since.get_or_insert_with(Instant::now);
    }

    /// Records a text change: marks the note dirty, relayouts and redraws.
    fn after_edit(&mut self) {
        let now = Instant::now();
        self.dirty_since.get_or_insert(now);
        self.last_edit = now;
        self.relayout();
        self.ensure_cursor_visible();
        self.needs_draw = true;
    }

    /// Applies one input-method update: replace the composition, insert committed text, then
    /// show the new composition at the cursor.
    fn ime_done(&mut self) {
        let v0 = self.editor.version();
        self.clear_composition();
        if let Some(text) = self.pending_commit.take() {
            self.editor.insert(&text);
        }
        let preedit = std::mem::take(&mut self.pending_preedit);
        if !preedit.is_empty() {
            self.editor.insert(&preedit);
            self.composing = preedit.chars().count();
        }
        if self.editor.version() != v0 {
            self.after_edit();
        }
        self.needs_draw = true;
    }

    // ---- keyboard ----

    /// Moves the cursor `steps` visual lines (negative = up), keeping the horizontal position.
    fn vertical(&mut self, steps: i32, extend: bool) {
        let gx = self
            .goal_x
            .unwrap_or_else(|| self.layout.x_of(self.editor.cursor()));
        self.goal_x = Some(gx);
        let mut idx = self.editor.cursor();
        for _ in 0..steps.unsigned_abs() {
            idx = if steps < 0 {
                self.layout.up(idx, gx)
            } else {
                self.layout.down(idx, gx)
            };
        }
        self.editor.set_cursor(idx, extend);
    }

    fn page_lines(&self) -> i32 {
        ((self.view_height() / self.fonts.line_height) as i32 - 1).max(1)
    }

    fn handle_key(&mut self, event: &KeyEvent) {
        // A key that reaches us while text is composing ends the composition.
        if self.composing > 0 {
            self.clear_composition();
            self.relayout();
        }
        let v0 = self.editor.version();
        let (ctrl, shift, alt, logo) = (
            self.mods.ctrl,
            self.mods.shift,
            self.mods.alt,
            self.mods.logo,
        );
        let mut keep_goal = false;

        match event.keysym {
            Keysym::Escape => {
                self.close();
                return;
            }
            Keysym::Left => {
                if ctrl {
                    self.editor.word_left(shift);
                } else {
                    self.editor.move_left(shift);
                }
            }
            Keysym::Right => {
                if ctrl {
                    self.editor.word_right(shift);
                } else {
                    self.editor.move_right(shift);
                }
            }
            Keysym::Up => {
                self.vertical(-1, shift);
                keep_goal = true;
            }
            Keysym::Down => {
                self.vertical(1, shift);
                keep_goal = true;
            }
            Keysym::Page_Up => {
                self.vertical(-self.page_lines(), shift);
                keep_goal = true;
            }
            Keysym::Page_Down => {
                self.vertical(self.page_lines(), shift);
                keep_goal = true;
            }
            Keysym::Home => {
                let i = if ctrl {
                    0
                } else {
                    self.layout.line_start_idx(self.editor.cursor())
                };
                self.editor.set_cursor(i, shift);
            }
            Keysym::End => {
                let i = if ctrl {
                    self.editor.len()
                } else {
                    self.layout.line_end_idx(self.editor.cursor())
                };
                self.editor.set_cursor(i, shift);
            }
            Keysym::BackSpace => {
                if ctrl {
                    self.editor.backspace_word();
                } else {
                    self.editor.backspace();
                }
            }
            Keysym::Delete => {
                if ctrl {
                    self.editor.delete_word();
                } else {
                    self.editor.delete_forward();
                }
            }
            Keysym::Return | Keysym::KP_Enter => self.editor.insert("\n"),
            Keysym::Tab => self.editor.insert("\t"),
            sym if ctrl && !alt => match sym.key_char().map(|c| c.to_ascii_lowercase()) {
                Some('a') => self.editor.select_all(),
                Some('z') => {
                    if shift {
                        self.editor.redo();
                    } else {
                        self.editor.undo();
                    }
                }
                Some('y') => {
                    self.editor.redo();
                }
                _ => {}
            },
            _ => {
                if !ctrl && !alt && !logo {
                    if let Some(t) = &event.utf8 {
                        self.editor.insert(t);
                    }
                }
            }
        }

        if !keep_goal {
            self.goal_x = None;
        }
        if self.editor.version() != v0 {
            let now = Instant::now();
            self.dirty_since.get_or_insert(now);
            self.last_edit = now;
            self.relayout();
        }
        self.ensure_cursor_visible();
        self.needs_draw = true;
    }

    // ---- mouse ----

    /// The cursor position nearest to the mouse pointer.
    fn hit_index(&self) -> usize {
        let s = self.scale.max(1) as f32;
        let x = self.mouse.0 as f32 * s - MARGIN * s;
        let y = self.mouse.1 as f32 * s - MARGIN * s + self.scroll_y;
        let line = (y / self.fonts.line_height).floor().max(0.0) as usize;
        self.layout.idx_at(line, x)
    }

    fn cursor_moved(&mut self) {
        self.goal_x = None;
        self.ensure_cursor_visible();
        self.needs_draw = true;
    }

    fn mouse_press(&mut self, time: u32) {
        let idx = self.hit_index();
        let close_to_last = self.last_click.is_some_and(|(t, p)| {
            time.wrapping_sub(t) < DOUBLE_CLICK_MS
                && (p.0 - self.mouse.0).abs() < 6.0
                && (p.1 - self.mouse.1).abs() < 6.0
        });
        self.click_count = if close_to_last {
            self.click_count % 3 + 1
        } else {
            1
        };
        self.last_click = Some((time, self.mouse));
        match self.click_count {
            1 => {
                self.editor.set_cursor(idx, self.mods.shift);
                self.dragging = true;
            }
            2 => {
                let (a, b) = text::word_range(self.editor.chars(), idx);
                self.editor.select_range(a, b);
                self.dragging = false;
            }
            _ => {
                let (a, b) = text::line_range(self.editor.chars(), idx);
                self.editor.select_range(a, b);
                self.dragging = false;
            }
        }
        self.cursor_moved();
    }

    // ---- drawing ----

    fn draw(&mut self) {
        self.needs_draw = false;
        let scale = self.scale.max(1) as u32;
        let (bw, bh) = (self.width * scale, self.height * scale);
        let stride = bw as i32 * 4;

        let buffer = self.buffer.get_or_insert_with(|| {
            self.pool
                .create_buffer(bw as i32, bh as i32, stride, wl_shm::Format::Argb8888)
                .expect("create buffer")
                .0
        });
        let canvas = match self.pool.canvas(buffer) {
            Some(canvas) => canvas,
            None => {
                // The compositor has not released the previous buffer yet: use a second one.
                let (second, canvas) = self
                    .pool
                    .create_buffer(bw as i32, bh as i32, stride, wl_shm::Format::Argb8888)
                    .expect("create buffer");
                *buffer = second;
                canvas
            }
        };

        let mut cv = Canvas {
            data: canvas,
            w: bw as i32,
            h: bh as i32,
        };
        render(
            &mut cv,
            scale as f32,
            &self.editor,
            &self.layout,
            &mut self.fonts,
            self.scroll_y,
        );

        // Tell the input method where the caret is, so its candidate list appears next to it.
        if self.ime_active {
            if let Some(text_input) = &self.text_input {
                let s = scale as f32;
                let (line, col) = self.layout.locate(self.editor.cursor());
                let x = (MARGIN * s + self.layout.lines[line].xs[col]) / s;
                let y = (MARGIN * s + line as f32 * self.fonts.line_height - self.scroll_y) / s;
                text_input.set_cursor_rectangle(
                    x as i32,
                    y as i32,
                    1,
                    (self.fonts.line_height / s) as i32,
                );
                text_input.commit();
            }
        }

        let surface = self.window.wl_surface();
        surface.set_buffer_scale(self.scale);
        surface.damage_buffer(0, 0, bw as i32, bh as i32);
        buffer.attach_to(surface).expect("buffer attach");
        self.window.commit();
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        self.set_scale(new_factor);
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl WindowHandler for App {
    fn request_close(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _window: &Window) {
        self.close();
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _window: &Window,
        configure: WindowConfigure,
        _serial: u32,
    ) {
        let w = configure.new_size.0.map_or(self.width, |v| v.get());
        let h = configure.new_size.1.map_or(self.height, |v| v.get());
        if !self.configured || w != self.width || h != self.height {
            self.width = w;
            self.height = h;
            self.buffer = None;
            self.relayout();
            self.ensure_cursor_visible();
        }
        self.configured = true;
        self.needs_draw = true;
    }
}

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            match self.seat_state.get_keyboard(qh, &seat, None) {
                Ok(keyboard) => self.keyboard = Some(keyboard),
                Err(e) => eprintln!("{APP_ID}: could not get the keyboard: {e}"),
            }
            if self.text_input.is_none() {
                if let Some(manager) = &self.text_manager {
                    self.text_input = Some(manager.get_text_input(&seat, qh, ()));
                }
            }
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            match self.seat_state.get_pointer(qh, &seat) {
                Ok(pointer) => self.pointer = Some(pointer),
                Err(e) => eprintln!("{APP_ID}: could not get the pointer: {e}"),
            }
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard {
            if let Some(keyboard) = self.keyboard.take() {
                keyboard.release();
            }
            self.repeat = None;
        }
        if capability == Capability::Pointer {
            if let Some(pointer) = self.pointer.take() {
                pointer.release();
            }
        }
    }

    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {
    }
}

impl KeyboardHandler for App {
    fn enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _serial: u32,
        _raw: &[u32],
        _keysyms: &[Keysym],
    ) {
    }

    fn leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _serial: u32,
    ) {
        self.repeat = None;
    }

    fn press_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        self.handle_key(&event);
        if !self.exit && !event.keysym.is_modifier_key() {
            self.repeat = Some(Repeat {
                event,
                next: Instant::now() + self.repeat_delay,
            });
        }
    }

    fn release_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        if self
            .repeat
            .as_ref()
            .is_some_and(|r| r.event.raw_code == event.raw_code)
        {
            self.repeat = None;
        }
    }

    fn update_modifiers(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        modifiers: Modifiers,
        _layout: u32,
    ) {
        self.mods = modifiers;
    }

    fn update_repeat_info(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        info: RepeatInfo,
    ) {
        match info {
            RepeatInfo::Repeat { rate, delay } => {
                self.repeat_delay = Duration::from_millis(delay as u64);
                self.repeat_interval =
                    Some(Duration::from_millis((1000 / rate.get()).max(1) as u64));
            }
            RepeatInfo::Disable => self.repeat_interval = None,
        }
    }
}

impl PointerHandler for App {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            if &event.surface != self.window.wl_surface() {
                continue;
            }
            self.mouse = event.position;
            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Leave { .. } => {}
                PointerEventKind::Motion { .. } => {
                    if self.dragging {
                        let idx = self.hit_index();
                        self.editor.set_cursor(idx, true);
                        self.cursor_moved();
                    }
                }
                PointerEventKind::Press { time, button, .. } => {
                    if button == BTN_LEFT {
                        self.mouse_press(time);
                    }
                }
                PointerEventKind::Release { button, .. } => {
                    if button == BTN_LEFT {
                        self.dragging = false;
                    }
                }
                PointerEventKind::Axis { vertical, .. } => {
                    // Wheel notches scroll three lines each; touchpads give a smooth amount.
                    let delta = if vertical.discrete != 0 {
                        vertical.discrete as f32 * 3.0 * self.fonts.line_height
                    } else {
                        vertical.absolute as f32 * self.scale.max(1) as f32
                    };
                    self.scroll_y += delta;
                    self.clamp_scroll();
                    self.needs_draw = true;
                }
            }
        }
    }
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_registry!(App);

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState,];
}

impl Dispatch<zwp_text_input_manager_v3::ZwpTextInputManagerV3, ()> for App {
    fn event(
        _: &mut Self,
        _: &zwp_text_input_manager_v3::ZwpTextInputManagerV3,
        _: zwp_text_input_manager_v3::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// Events from the input method (IME), for example fcitx or ibus.
impl Dispatch<zwp_text_input_v3::ZwpTextInputV3, ()> for App {
    fn event(
        app: &mut Self,
        text_input: &zwp_text_input_v3::ZwpTextInputV3,
        event: zwp_text_input_v3::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwp_text_input_v3::Event;
        match event {
            Event::Enter { .. } => {
                app.ime_active = true;
                text_input.enable();
                text_input.commit();
                app.needs_draw = true;
            }
            Event::Leave { .. } => {
                app.ime_active = false;
                app.pending_commit = None;
                app.pending_preedit.clear();
                let v0 = app.editor.version();
                app.clear_composition();
                if app.editor.version() != v0 {
                    app.after_edit();
                }
                text_input.disable();
                text_input.commit();
            }
            Event::PreeditString { text, .. } => app.pending_preedit = text.unwrap_or_default(),
            Event::CommitString { text } => app.pending_commit = text,
            Event::Done { .. } => app.ime_done(),
            _ => {}
        }
    }
}

delegate_compositor!(App);
delegate_output!(App);
delegate_shm!(App);
delegate_seat!(App);
delegate_keyboard!(App);
delegate_pointer!(App);
delegate_xdg_shell!(App);
delegate_xdg_window!(App);

fn print_usage() {
    println!(
        "hyprsticky: minimal sticky notes for Hyprland\n\n\
         usage:\n  \
         hyprsticky           open a new, empty note\n  \
         hyprsticky <name>    open (or create) the note with that name\n  \
         hyprsticky list      list saved notes, newest first\n\n\
         environment:\n  \
         HYPRSTICKY_DIR   folder where notes are stored (default ~/.local/share/hyprsticky)\n  \
         HYPRSTICKY_FONT  fontconfig pattern or font file path (default sans-serif)"
    );
}

fn main() {
    let dir = store::notes_dir();
    let arg = std::env::args().nth(1);

    match arg.as_deref() {
        Some("list" | "--list" | "-l") => {
            store::list_notes(&dir);
            return;
        }
        Some("help" | "--help" | "-h") => {
            print_usage();
            return;
        }
        _ => {}
    }

    // No argument: a brand-new, empty note. With a name: open (or create) that note.
    let id = match arg.map(|n| store::clean_name(&n)).filter(|n| !n.is_empty()) {
        Some(name) => name,
        None => store::new_note_id(&dir),
    };
    let path = store::note_path(&dir, &id);
    let mut editor = Editor::new(&store::read_note(&path));
    editor.set_cursor(usize::MAX, false);

    let font_spec = std::env::var("HYPRSTICKY_FONT").unwrap_or_default();
    let fonts = Fonts::new(&font_spec).unwrap_or_else(|e| {
        eprintln!("{APP_ID}: {e}");
        std::process::exit(1);
    });

    let conn = Connection::connect_to_env().unwrap_or_else(|e| {
        eprintln!("{APP_ID}: could not connect to Wayland: {e}");
        std::process::exit(1);
    });
    let (globals, event_queue) = registry_queue_init(&conn).unwrap_or_else(|e| {
        eprintln!("{APP_ID}: could not read the Wayland globals: {e}");
        std::process::exit(1);
    });
    let qh = event_queue.handle();
    let mut event_loop: EventLoop<App> = EventLoop::try_new().expect("could not create event loop");
    WaylandSource::new(conn.clone(), event_queue)
        .insert(event_loop.handle())
        .expect("could not add the Wayland source");

    let compositor = CompositorState::bind(&globals, &qh).expect("wl_compositor not available");
    let xdg_shell = XdgShell::bind(&globals, &qh).expect("xdg shell not available");
    let shm = Shm::bind(&globals, &qh).expect("wl_shm not available");
    // Optional: without it the note still works, just without input-method support.
    let text_manager = globals
        .bind::<zwp_text_input_manager_v3::ZwpTextInputManagerV3, _, _>(&qh, 1..=1, ())
        .ok();

    let surface = compositor.create_surface(&qh);
    let window = xdg_shell.create_window(surface, WindowDecorations::None, &qh);
    window.set_title(format!("{APP_ID}: {id}"));
    window.set_app_id(APP_ID);
    window.set_min_size(Some((120, 80)));
    // The compositor answers this initial commit with a configure event; we draw on that.
    window.commit();

    let pool = SlotPool::new((DEFAULT_SIZE * DEFAULT_SIZE * 4) as usize, &shm)
        .expect("could not create the shared memory pool");

    let mut app = App {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        window,
        pool,
        buffer: None,
        keyboard: None,
        pointer: None,
        width: DEFAULT_SIZE,
        height: DEFAULT_SIZE,
        scale: 1,
        configured: false,
        needs_draw: false,
        exit: false,
        finished: false,
        path,
        editor,
        layout: Layout { lines: Vec::new() },
        fonts,
        scroll_y: 0.0,
        goal_x: None,
        dirty_since: None,
        last_edit: Instant::now(),
        mods: Modifiers::default(),
        repeat: None,
        repeat_delay: Duration::from_millis(400),
        repeat_interval: Some(Duration::from_millis(30)),
        mouse: (0.0, 0.0),
        dragging: false,
        click_count: 0,
        last_click: None,
        text_manager,
        text_input: None,
        ime_active: false,
        composing: 0,
        pending_commit: None,
        pending_preedit: String::new(),
    };
    app.relayout();

    loop {
        let timeout = app.next_timeout();
        if let Err(e) = event_loop.dispatch(timeout, &mut app) {
            // The compositor went away: keep what was typed.
            eprintln!("{APP_ID}: lost the Wayland connection: {e}");
            app.finish();
            break;
        }
        app.tick();
        if app.exit {
            break;
        }
        if app.needs_draw && app.configured {
            app.draw();
        }
    }
}
