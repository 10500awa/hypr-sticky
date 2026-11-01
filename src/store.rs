//! Where notes live on disk, how they are named, and saving/renaming. Only uses `std`.

use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const APP_ID: &str = "hyprsticky";
/// Notes are markdown files, so tools like Obsidian can open them.
pub const EXT: &str = "md";

/// Where notes live: $HYPRSTICKY_DIR if set (e.g. a folder inside an Obsidian vault),
/// otherwise ~/.local/share/hyprsticky (respects XDG_DATA_HOME).
pub fn notes_dir() -> PathBuf {
    if let Some(dir) = env::var_os("HYPRSTICKY_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_ID)
}

/// Turns text into a safe file name: drops characters that are invalid in file names or break
/// Obsidian links, collapses whitespace, and trims leading/trailing dots and spaces.
pub fn clean_name(s: &str) -> String {
    let kept: String =
        s.chars().filter(|c| !c.is_control() && !"\\/:*?\"<>|#^[]".contains(*c)).collect();
    let joined = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    joined.trim_matches('.').trim().to_string()
}

/// A fresh id for a new note: the current time in milliseconds.
pub fn new_note_id(dir: &Path) -> String {
    let mut n = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    while dir.join(format!("{n}.{EXT}")).exists() {
        n += 1;
    }
    n.to_string()
}

pub fn note_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.{EXT}"))
}

pub fn read_note(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

/// Saves the note. An empty note leaves no file behind.
pub fn write_note(path: &Path, text: &str) {
    if text.is_empty() {
        let _ = fs::remove_file(path);
        return;
    }
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Err(e) = fs::write(path, text) {
        eprintln!("{APP_ID}: could not save {}: {e}", path.display());
    }
}

/// A note that still has its timestamp name is renamed after its first line (e.g.
/// "Buy milk.md"). Notes with any other name are left alone, and an existing note is never
/// overwritten. Returns the new path if the note was renamed.
pub fn rename_to_title(path: &Path, text: &str) -> Option<PathBuf> {
    let is_timestamp_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()));
    if text.is_empty() || !is_timestamp_name || !path.exists() {
        return None;
    }
    let dir = path.parent()?;

    let first_line = text.lines().find(|l| !l.trim().is_empty())?;
    let raw: String = clean_name(first_line).chars().take(60).collect();
    let title = raw.trim_end_matches(['.', ' ']);
    if title.is_empty() {
        return None;
    }

    let mut target = dir.join(format!("{title}.{EXT}"));
    let mut n = 2;
    while target.exists() {
        target = dir.join(format!("{title} {n}.{EXT}"));
        n += 1;
    }
    fs::rename(path, &target).ok()?;
    Some(target)
}

/// Prints saved notes, newest first: id, then the first line.
pub fn list_notes(dir: &Path) {
    let mut notes: Vec<_> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == EXT))
        .map(|e| (e.metadata().and_then(|m| m.modified()).unwrap_or(UNIX_EPOCH), e.path()))
        .collect();
    notes.sort_by(|a, b| b.cmp(a));
    for (_, path) in notes {
        let text = read_note(&path);
        let first_line: String = text.lines().next().unwrap_or("").chars().take(60).collect();
        println!("{}\t{first_line}", path.file_stem().unwrap_or_default().to_string_lossy());
    }
}
