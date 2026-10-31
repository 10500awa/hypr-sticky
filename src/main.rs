use eframe::egui;
use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const APP_ID: &str = "hyprsticky";
/// Notes are markdown files, so tools like Obsidian can open them.
const EXT: &str = "md";
const PAPER: egui::Color32 = egui::Color32::from_rgb(255, 241, 118);
const INK: egui::Color32 = egui::Color32::from_rgb(45, 40, 20);

/// One window = one note, stored as a markdown file.
struct Sticky {
    path: PathBuf,
    text: String,
    focused: bool,
}

impl Sticky {
    /// Saves the note. An empty note leaves no file behind.
    fn write_note(&self) {
        if self.text.is_empty() {
            let _ = fs::remove_file(&self.path);
            return;
        }
        if let Some(dir) = self.path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Err(e) = fs::write(&self.path, &self.text) {
            eprintln!("{APP_ID}: could not save {}: {e}", self.path.display());
        }
    }
}

impl eframe::App for Sticky {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Ctrl+W closes the note (it is already saved).
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::W)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(PAPER)
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ctx, |ui| {
                ui.visuals_mut().override_text_color = Some(INK);
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        let resp = ui.add_sized(
                            ui.available_size(),
                            egui::TextEdit::multiline(&mut self.text).frame(false),
                        );
                        if !self.focused {
                            resp.request_focus();
                            self.focused = true;
                        }
                        if resp.changed() {
                            self.write_note();
                        }
                    });
            });
    }
}

/// Where notes live: $HYPRSTICKY_DIR if set (e.g. a folder inside an Obsidian vault),
/// otherwise ~/.local/share/hyprsticky (respects XDG_DATA_HOME).
fn notes_dir() -> PathBuf {
    if let Some(dir) = env::var_os("HYPRSTICKY_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_ID)
}

/// Keeps note names safe to use as file names.
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// A fresh id for a new note: the current time in milliseconds.
fn new_note_id(dir: &Path) -> String {
    let mut n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    while dir.join(format!("{n}.{EXT}")).exists() {
        n += 1;
    }
    n.to_string()
}

/// Prints saved notes, newest first: id, then the first line.
fn list_notes(dir: &Path) {
    let mut notes = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some(EXT) {
                continue;
            }
            let id = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let first_line: String = fs::read_to_string(&path)
                .unwrap_or_default()
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(60)
                .collect();
            let modified = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(UNIX_EPOCH);
            notes.push((modified, id, first_line));
        }
    }
    notes.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, id, first_line) in notes {
        println!("{id}\t{first_line}");
    }
}

fn main() -> eframe::Result {
    let dir = notes_dir();
    let arg = env::args().nth(1);

    if matches!(arg.as_deref(), Some("list" | "--list" | "-l")) {
        list_notes(&dir);
        return Ok(());
    }

    // No argument: a brand-new, empty note. With an id/name: open (or create) that note.
    let id = match arg {
        Some(name) => sanitize(&name),
        None => new_note_id(&dir),
    };
    let path = dir.join(format!("{id}.{EXT}"));
    let text = fs::read_to_string(&path).unwrap_or_default();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id(APP_ID)
            .with_title(format!("{APP_ID}: {id}"))
            .with_inner_size([260.0, 260.0])
            .with_decorations(false),
        ..Default::default()
    };

    eframe::run_native(
        APP_ID,
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::light());
            Ok(Box::new(Sticky {
                path,
                text,
                focused: false,
            }))
        }),
    )
}
