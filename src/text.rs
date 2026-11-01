//! Plain-text buffer with a cursor, a selection and undo/redo. No dependencies.
//!
//! The text is stored as `Vec<char>` and every position is a char index, which keeps cursor
//! movement and hit-testing simple. Notes are small, so the extra memory is negligible.

const MAX_UNDO: usize = 200;

/// A saved state: (text, cursor, anchor).
type Snapshot = (Vec<char>, usize, usize);

/// What kind of edit happened last; consecutive edits of the same kind share one undo step.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Insert,
    Backspace,
    Delete,
    Other,
}

#[derive(Default)]
pub struct Editor {
    chars: Vec<char>,
    cursor: usize,
    anchor: usize,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last: Option<Kind>,
    version: u64,
}

#[derive(PartialEq)]
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
fn prev_word(chars: &[char], idx: usize) -> usize {
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
fn next_word(chars: &[char], idx: usize) -> usize {
    let n = chars.len();
    let mut i = idx.min(n);
    if i < n && class(chars[i]) != Class::Space {
        let cl = class(chars[i]);
        while i < n && class(chars[i]) == cl {
            i += 1;
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
    let a = chars[..i].iter().rposition(|&c| class(c) != cl).map_or(0, |p| p + 1);
    let b = chars[i..].iter().position(|&c| class(c) != cl).map_or(chars.len(), |p| i + p);
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
    s.chars().filter(|&c| c == '\n' || c == '\t' || !c.is_control()).collect()
}

impl Editor {
    pub fn new(text: &str) -> Editor {
        Editor { chars: clean(text), ..Default::default() }
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

    /// The selection as (start, end); empty (start == end) when nothing is selected.
    fn span(&self) -> (usize, usize) {
        (self.anchor.min(self.cursor), self.anchor.max(self.cursor))
    }

    /// The selected range as (start, end), if any.
    pub fn selection(&self) -> Option<(usize, usize)> {
        Some(self.span()).filter(|(a, b)| a != b)
    }

    /// Call before changing the text: records an undo step unless this edit continues the
    /// previous one.
    fn checkpoint(&mut self, kind: Kind, force_new: bool) {
        if force_new || kind == Kind::Other || self.last != Some(kind) {
            self.undo.push((self.chars.clone(), self.cursor, self.anchor));
            if self.undo.len() > MAX_UNDO {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last = Some(kind);
    }

    /// Replaces `a..b` with `new` and puts the cursor after it.
    fn replace(&mut self, a: usize, b: usize, new: &[char]) {
        self.chars.splice(a..b, new.iter().copied());
        self.cursor = a + new.len();
        self.anchor = self.cursor;
        self.version += 1;
    }

    /// Moves one step through the history: back (`undo`) or forward.
    fn travel(&mut self, undo: bool) {
        let (from, to) =
            if undo { (&mut self.undo, &mut self.redo) } else { (&mut self.redo, &mut self.undo) };
        let Some((chars, cursor, anchor)) = from.pop() else { return };
        to.push((std::mem::replace(&mut self.chars, chars), self.cursor, self.anchor));
        (self.cursor, self.anchor) = (cursor, anchor);
        self.version += 1;
        self.last = None;
    }

    pub fn undo(&mut self) {
        self.travel(true);
    }

    pub fn redo(&mut self) {
        self.travel(false);
    }

    /// Types text at the cursor, replacing the selection if there is one.
    pub fn insert(&mut self, s: &str) {
        let new = clean(s);
        if new.is_empty() {
            return;
        }
        let (a, b) = self.span();
        // Start a new undo step at the start of each word, at newlines and when replacing.
        let after_space = self.cursor > 0 && self.chars[self.cursor - 1].is_whitespace();
        let force_new = a != b || new.contains(&'\n') || (after_space && !new[0].is_whitespace());
        self.checkpoint(Kind::Insert, force_new);
        self.replace(a, b, &new);
    }

    /// Input-method update: replaces the `old` chars before the cursor (the previous
    /// composition) with `commit` followed by `preedit`, the new composition. A whole
    /// composition is one undo step. Returns how many chars of `preedit` were inserted.
    pub fn compose(&mut self, old: usize, commit: &str, preedit: &str) -> usize {
        let (mut new, preedit) = (clean(commit), clean(preedit));
        if old == 0 && new.is_empty() && preedit.is_empty() {
            return 0;
        }
        new.extend_from_slice(&preedit);
        let (a, b) = if old == 0 {
            self.checkpoint(Kind::Other, true);
            self.span()
        } else {
            (self.cursor.saturating_sub(old), self.cursor)
        };
        self.replace(a, b, &new);
        preedit.len()
    }

    fn erase(&mut self, a: usize, b: usize, kind: Kind) {
        if a < b {
            self.checkpoint(kind, false);
            self.replace(a, b, &[]);
        }
    }

    /// Deletes the selection, or else the char (with `word`: the word) before the cursor.
    pub fn backspace(&mut self, word: bool) {
        let c = self.cursor;
        let (a, b, kind) = match self.selection() {
            Some((a, b)) => (a, b, Kind::Other),
            None if word => (prev_word(&self.chars, c), c, Kind::Other),
            None => (c.saturating_sub(1), c, Kind::Backspace),
        };
        self.erase(a, b, kind);
    }

    /// Deletes the selection, or else the char (with `word`: the word) after the cursor.
    pub fn delete(&mut self, word: bool) {
        let c = self.cursor;
        let (a, b, kind) = match self.selection() {
            Some((a, b)) => (a, b, Kind::Other),
            None if word => (c, next_word(&self.chars, c), Kind::Other),
            None => (c, (c + 1).min(self.chars.len()), Kind::Delete),
        };
        self.erase(a, b, kind);
    }

    /// Moves the cursor to `idx`; with `extend` the selection grows from where it started.
    pub fn set_cursor(&mut self, idx: usize, extend: bool) {
        self.cursor = idx.min(self.chars.len());
        if !extend {
            self.anchor = self.cursor;
        }
        self.last = None;
    }

    /// Cursor left by a char (with `word`: a word). Without `extend`, a selection collapses.
    pub fn left(&mut self, word: bool, extend: bool) {
        match self.selection() {
            Some((a, _)) if !extend && !word => self.set_cursor(a, false),
            _ if word => self.set_cursor(prev_word(&self.chars, self.cursor), extend),
            _ => self.set_cursor(self.cursor.saturating_sub(1), extend),
        }
    }

    /// Cursor right by a char (with `word`: a word). Without `extend`, a selection collapses.
    pub fn right(&mut self, word: bool, extend: bool) {
        match self.selection() {
            Some((_, b)) if !extend && !word => self.set_cursor(b, false),
            _ if word => self.set_cursor(next_word(&self.chars, self.cursor), extend),
            _ => self.set_cursor(self.cursor + 1, extend),
        }
    }

    pub fn select_all(&mut self) {
        self.select_range(0, self.chars.len());
    }

    pub fn select_range(&mut self, a: usize, b: usize) {
        let n = self.chars.len();
        (self.anchor, self.cursor) = (a.min(n), b.min(n));
        self.last = None;
    }
}
