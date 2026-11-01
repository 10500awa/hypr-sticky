//! Plain-text buffer with a cursor, a selection and undo/redo. No dependencies.
//!
//! The text is stored as `Vec<char>` and every position is a char index, which keeps cursor
//! movement and hit-testing simple. Notes are small, so the extra memory is negligible.

const MAX_UNDO: usize = 200;

#[derive(Clone)]
struct Snapshot {
    chars: Vec<char>,
    cursor: usize,
    anchor: Option<usize>,
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
    anchor: Option<usize>,
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
    let mut a = idx;
    while a > 0 && chars[a - 1] != '\n' {
        a -= 1;
    }
    let mut b = idx;
    while b < chars.len() && chars[b] != '\n' {
        b += 1;
    }
    if b < chars.len() {
        b += 1;
    }
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
            anchor: None,
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

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
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
        self.anchor
            .filter(|&a| a != self.cursor)
            .map(|a| (a.min(self.cursor), a.max(self.cursor)))
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
        self.anchor = s.anchor.filter(|&a| a <= self.chars.len());
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
        self.anchor = None;
        self.version += 1;
    }

    pub fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some(s) => {
                self.redo.push(self.snapshot());
                self.restore(s);
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some(s) => {
                self.undo.push(self.snapshot());
                self.restore(s);
                true
            }
            None => false,
        }
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
        self.anchor = None;
        self.version += 1;
    }

    fn delete_selection(&mut self) -> bool {
        match self.selection() {
            Some((a, b)) => {
                self.checkpoint(Kind::Other, true);
                self.remove_range(a, b);
                true
            }
            None => false,
        }
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
        let idx = idx.min(self.chars.len());
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor);
            }
        } else {
            self.anchor = None;
        }
        self.cursor = idx;
        if self.anchor == Some(self.cursor) {
            self.anchor = None;
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
        let i = prev_word(&self.chars, self.cursor);
        self.set_cursor(i, extend);
    }

    pub fn word_right(&mut self, extend: bool) {
        let i = next_word(&self.chars, self.cursor);
        self.set_cursor(i, extend);
    }

    pub fn select_all(&mut self) {
        self.select_range(0, self.chars.len());
    }

    pub fn select_range(&mut self, a: usize, b: usize) {
        let n = self.chars.len();
        self.anchor = Some(a.min(n));
        self.cursor = b.min(n);
        if self.anchor == Some(self.cursor) {
            self.anchor = None;
        }
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ed(s: &str) -> Editor {
        Editor::new(s)
    }

    #[test]
    fn typing_and_text() {
        let mut e = ed("");
        e.insert("hello");
        e.insert(" world");
        assert_eq!(e.text(), "hello world");
        assert_eq!(e.cursor(), 11);
    }

    #[test]
    fn insert_filters_control_chars_and_cr() {
        let mut e = ed("a\r\nb\u{1}");
        assert_eq!(e.text(), "a\nb");
        e.insert("\u{7}x\r");
        assert_eq!(e.text(), "xa\nb");
    }

    #[test]
    fn accents_and_cjk_are_single_chars() {
        let mut e = ed("");
        e.insert("á");
        e.insert("圍道");
        assert_eq!(e.len(), 3);
        e.backspace();
        assert_eq!(e.text(), "á圍");
    }

    #[test]
    fn backspace_and_delete() {
        let mut e = ed("abc");
        e.set_cursor(3, false);
        e.backspace();
        assert_eq!(e.text(), "ab");
        e.set_cursor(0, false);
        e.delete_forward();
        assert_eq!(e.text(), "b");
        // Backspace at the start and delete at the end do nothing.
        e.backspace();
        assert_eq!(e.text(), "b");
        e.set_cursor(1, false);
        e.delete_forward();
        assert_eq!(e.text(), "b");
    }

    #[test]
    fn selection_replaced_by_typing() {
        let mut e = ed("hello world");
        e.select_range(0, 5);
        assert_eq!(e.selection(), Some((0, 5)));
        e.insert("bye");
        assert_eq!(e.text(), "bye world");
        assert_eq!(e.selection(), None);
        assert_eq!(e.cursor(), 3);
    }

    #[test]
    fn selection_deleted_by_backspace() {
        let mut e = ed("hello world");
        e.select_range(6, 11);
        e.backspace();
        assert_eq!(e.text(), "hello ");
    }

    #[test]
    fn shift_selection_and_collapse() {
        let mut e = ed("abcdef");
        e.set_cursor(2, false);
        e.move_right(true);
        e.move_right(true);
        assert_eq!(e.selection(), Some((2, 4)));
        e.move_left(false);
        assert_eq!(e.cursor(), 2);
        assert_eq!(e.selection(), None);
        e.select_range(1, 4);
        e.move_right(false);
        assert_eq!(e.cursor(), 4);
    }

    #[test]
    fn select_back_to_anchor_clears_selection() {
        let mut e = ed("abc");
        e.set_cursor(1, false);
        e.move_right(true);
        e.move_left(true);
        assert_eq!(e.selection(), None);
    }

    #[test]
    fn word_motion() {
        let s: Vec<char> = "foo  bar.baz".chars().collect();
        assert_eq!(next_word(&s, 0), 5);
        assert_eq!(next_word(&s, 5), 8);
        assert_eq!(next_word(&s, 8), 9);
        assert_eq!(prev_word(&s, 12), 9);
        assert_eq!(prev_word(&s, 9), 8);
        assert_eq!(prev_word(&s, 8), 5);
        assert_eq!(prev_word(&s, 5), 0);
        assert_eq!(prev_word(&s, 0), 0);
        assert_eq!(next_word(&s, 12), 12);
    }

    #[test]
    fn word_and_line_ranges() {
        let s: Vec<char> = "foo bar\nbaz".chars().collect();
        assert_eq!(word_range(&s, 1), (0, 3));
        assert_eq!(word_range(&s, 3), (3, 4));
        assert_eq!(word_range(&s, 100), (8, 11));
        assert_eq!(line_range(&s, 2), (0, 8));
        assert_eq!(line_range(&s, 9), (8, 11));
        assert_eq!(word_range(&[], 0), (0, 0));
    }

    #[test]
    fn word_deletes() {
        let mut e = ed("foo bar");
        e.set_cursor(7, false);
        e.backspace_word();
        assert_eq!(e.text(), "foo ");
        e.set_cursor(0, false);
        e.delete_word();
        assert_eq!(e.text(), "");
    }

    #[test]
    fn undo_groups_by_word_and_redo() {
        let mut e = ed("");
        e.insert("a");
        e.insert("b");
        e.insert(" ");
        e.insert("c");
        e.insert("d");
        assert_eq!(e.text(), "ab cd");
        assert!(e.undo());
        assert_eq!(e.text(), "ab ");
        assert!(e.undo());
        assert_eq!(e.text(), "");
        assert!(!e.undo());
        assert!(e.redo());
        assert_eq!(e.text(), "ab ");
        assert!(e.redo());
        assert_eq!(e.text(), "ab cd");
        assert!(!e.redo());
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut e = ed("");
        e.insert("a");
        e.undo();
        e.insert("b");
        assert!(!e.redo());
        assert_eq!(e.text(), "b");
    }

    #[test]
    fn undo_restores_deleted_selection() {
        let mut e = ed("hello world");
        e.select_range(0, 5);
        e.backspace();
        assert_eq!(e.text(), " world");
        e.undo();
        assert_eq!(e.text(), "hello world");
        assert_eq!(e.selection(), Some((0, 5)));
    }

    #[test]
    fn moving_cursor_ends_undo_group() {
        let mut e = ed("");
        e.insert("ab");
        e.set_cursor(0, false);
        e.insert("x");
        assert_eq!(e.text(), "xab");
        e.undo();
        assert_eq!(e.text(), "ab");
        e.undo();
        assert_eq!(e.text(), "");
    }

    #[test]
    fn version_changes_only_on_text_changes() {
        let mut e = ed("abc");
        let v = e.version();
        e.set_cursor(2, false);
        e.move_left(true);
        assert_eq!(e.version(), v);
        e.insert("x");
        assert!(e.version() > v);
    }

    #[test]
    fn undo_history_is_bounded() {
        let mut e = ed("");
        for i in 0..(MAX_UNDO + 50) {
            e.insert(&format!("{i}\n"));
        }
        assert!(e.undo.len() <= MAX_UNDO);
    }
}
