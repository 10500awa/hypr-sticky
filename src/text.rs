//! Plain-text buffer with a cursor, a selection and undo/redo. No dependencies.
//!
//! The text is stored as `Vec<char>` and every position is a char index, which keeps cursor
//! movement and hit-testing simple. Notes are small, so the extra memory is negligible.

const MAX_UNDO: usize = 200;

#[derive(Clone)]
struct Snapshot {
    chars: Vec<char>,
    cursor: usize,
    anchor: usize,
}

/// What kind of edit happened last; consecutive edits of the same kind share one undo step.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Insert,
    Backspace,
    Delete,
    Other,
}

pub struct Editor {
    chars: Vec<char>,
    cursor: usize,
    anchor: usize,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last: Option<Kind>,
    version: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Space,
    Word,
    Punct,
}

fn class(c: char) -> Class {
    if c.is_whitespace() {
        Class::Space
    } else if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

/// Start of the word before `idx` (Ctrl+Left).
pub fn prev_word(chars: &[char], idx: usize) -> usize {
    let mut i = idx.min(chars.len());
    while i > 0 && class(chars[i - 1]) == Class::Space {
        i -= 1;
    }
    if i > 0 {
        let cl = class(chars[i - 1]);
        while i > 0 && class(chars[i - 1]) == cl {
            i -= 1;
        }
    }
    i
}

/// Start of the word after `idx` (Ctrl+Right).
pub fn next_word(chars: &[char], idx: usize) -> usize {
    let n = chars.len();
    let mut i = idx.min(n);
    if i < n {
        let cl = class(chars[i]);
        if cl != Class::Space {
            while i < n && class(chars[i]) == cl {
                i += 1;
            }
        }
    }
    while i < n && class(chars[i]) == Class::Space {
        i += 1;
    }
    i
}

/// The run of same-kind characters (word, spaces or punctuation) around `idx`.
pub fn word_range(chars: &[char], idx: usize) -> (usize, usize) {
    if chars.is_empty() {
        return (0, 0);
    }
    let i = idx.min(chars.len() - 1);
    let cl = class(chars[i]);
    let mut a = i;
    while a > 0 && class(chars[a - 1]) == cl {
        a -= 1;
    }
    let mut b = i + 1;
    while b < chars.len() && class(chars[b]) == cl {
        b += 1;
    }
    (a, b)
}

/// The whole logical line (up to and including its newline) around `idx`.
pub fn line_range(chars: &[char], idx: usize) -> (usize, usize) {
    let idx = idx.min(chars.len());
    let a = chars[..idx].iter().rposition(|&c| c == '\n').map_or(0, |p| p + 1);
    let b = chars[idx..].iter().position(|&c| c == '\n').map_or(chars.len(), |p| idx + p + 1);
    (a, b)
}

/// Keeps newlines, tabs and printable characters; drops other control characters and `\r`.
fn clean(s: &str) -> Vec<char> {
    s.chars()
        .filter(|&c| c == '\n' || c == '\t' || !c.is_control())
        .collect()
}

impl Editor {
    pub fn new(text: &str) -> Editor {
        Editor {
            chars: clean(text),
            cursor: 0,
            anchor: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            last: None,
            version: 0,
        }
    }

    pub fn chars(&self) -> &[char] {
        &self.chars
    }

    pub fn len(&self) -> usize {
        self.chars.len()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Increases every time the text changes (not for cursor movement).
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    /// The selected range as (start, end), if any.
    pub fn selection(&self) -> Option<(usize, usize)> {
        (self.anchor != self.cursor)
            .then(|| (self.anchor.min(self.cursor), self.anchor.max(self.cursor)))
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            chars: self.chars.clone(),
            cursor: self.cursor,
            anchor: self.anchor,
        }
    }

    fn restore(&mut self, s: Snapshot) {
        self.chars = s.chars;
        self.cursor = s.cursor.min(self.chars.len());
        self.anchor = s.anchor.min(self.chars.len());
        self.version += 1;
        self.last = None;
    }

    /// Call before changing the text: records an undo step unless this edit continues the
    /// previous one.
    fn checkpoint(&mut self, kind: Kind, force_new: bool) {
        if force_new || kind == Kind::Other || self.last != Some(kind) {
            self.undo.push(self.snapshot());
            if self.undo.len() > MAX_UNDO {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last = Some(kind);
    }

    fn remove_range(&mut self, a: usize, b: usize) {
        self.chars.drain(a..b);
        self.cursor = a;
        self.anchor = a;
        self.version += 1;
    }

    pub fn undo(&mut self) -> bool {
        let Some(s) = self.undo.pop() else {
            return false;
        };
        self.redo.push(self.snapshot());
        self.restore(s);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(s) = self.redo.pop() else {
            return false;
        };
        self.undo.push(self.snapshot());
        self.restore(s);
        true
    }

    /// Types text at the cursor, replacing the selection if there is one.
    pub fn insert(&mut self, s: &str) {
        let new = clean(s);
        if new.is_empty() {
            return;
        }
        let selection = self.selection();
        // Start a new undo step at the start of each word, at newlines and when replacing.
        let after_space = self.cursor > 0 && self.chars[self.cursor - 1].is_whitespace();
        let force_new =
            selection.is_some() || new.contains(&'\n') || (after_space && !new[0].is_whitespace());
        self.checkpoint(Kind::Insert, force_new);
        if let Some((a, b)) = selection {
            self.remove_range(a, b);
        }
        let at = self.cursor;
        let n = new.len();
        self.chars.splice(at..at, new);
        self.cursor = at + n;
        self.anchor = self.cursor;
        self.version += 1;
    }

    fn delete_selection(&mut self) -> bool {
        let Some((a, b)) = self.selection() else {
            return false;
        };
        self.checkpoint(Kind::Other, true);
        self.remove_range(a, b);
        true
    }

    pub fn backspace(&mut self) {
        if self.delete_selection() || self.cursor == 0 {
            return;
        }
        self.checkpoint(Kind::Backspace, false);
        self.remove_range(self.cursor - 1, self.cursor);
    }

    pub fn delete_forward(&mut self) {
        if self.delete_selection() || self.cursor >= self.chars.len() {
            return;
        }
        self.checkpoint(Kind::Delete, false);
        let c = self.cursor;
        self.remove_range(c, c + 1);
    }

    pub fn backspace_word(&mut self) {
        if self.delete_selection() {
            return;
        }
        let a = prev_word(&self.chars, self.cursor);
        if a < self.cursor {
            self.checkpoint(Kind::Other, true);
            self.remove_range(a, self.cursor);
        }
    }

    pub fn delete_word(&mut self) {
        if self.delete_selection() {
            return;
        }
        let b = next_word(&self.chars, self.cursor);
        if b > self.cursor {
            self.checkpoint(Kind::Other, true);
            let c = self.cursor;
            self.remove_range(c, b);
        }
    }

    /// Moves the cursor to `idx`; with `extend` the selection grows from where it started.
    pub fn set_cursor(&mut self, idx: usize, extend: bool) {
        self.cursor = idx.min(self.chars.len());
        if !extend {
            self.anchor = self.cursor;
        }
        self.last = None;
    }

    pub fn move_left(&mut self, extend: bool) {
        if !extend {
            if let Some((a, _)) = self.selection() {
                self.set_cursor(a, false);
                return;
            }
        }
        self.set_cursor(self.cursor.saturating_sub(1), extend);
    }

    pub fn move_right(&mut self, extend: bool) {
        if !extend {
            if let Some((_, b)) = self.selection() {
                self.set_cursor(b, false);
                return;
            }
        }
        self.set_cursor(self.cursor + 1, extend);
    }

    pub fn word_left(&mut self, extend: bool) {
        self.set_cursor(prev_word(&self.chars, self.cursor), extend);
    }

    pub fn word_right(&mut self, extend: bool) {
        self.set_cursor(next_word(&self.chars, self.cursor), extend);
    }

    pub fn select_all(&mut self) {
        self.select_range(0, self.chars.len());
    }

    pub fn select_range(&mut self, a: usize, b: usize) {
        let n = self.chars.len();
        self.anchor = a.min(n);
        self.cursor = b.min(n);
        self.last = None;
    }
}
