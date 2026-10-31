use eframe::egui;
use std::{env, fs, path::PathBuf};

const PAPER: egui::Color32 = egui::Color32::from_rgb(255, 241, 118);
const INK: egui::Color32 = egui::Color32::from_rgb(45, 40, 20);

/// One window = one note, stored as a plain text file.
struct Sticky {
    path: PathBuf,
    text: String,
    focused: bool,
}

impl Sticky {
    fn save(&self) {
        if let Some(dir) = self.path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Err(e) = fs::write(&self.path, &self.text) {
            eprintln!("hypr-sticky: could not save {}: {e}", self.path.display());
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
                            self.save();
                        }
                    });
            });
    }
}

/// ~/.local/share/hypr-sticky/<name>.txt (respects XDG_DATA_HOME).
fn note_path(name: &str) -> PathBuf {
    let base = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    base.join("hypr-sticky").join(format!("{safe}.txt"))
}

fn main() -> eframe::Result {
    let name = env::args().nth(1).unwrap_or_else(|| "note".to_string());
    let path = note_path(&name);
    let text = fs::read_to_string(&path).unwrap_or_default();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id("hypr-sticky")
            .with_title(format!("hypr-sticky: {name}"))
            .with_inner_size([260.0, 260.0])
            .with_decorations(false),
        ..Default::default()
    };

    eframe::run_native(
        "hypr-sticky",
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
