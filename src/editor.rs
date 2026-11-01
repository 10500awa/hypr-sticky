//! Text buffer, editing, selection, undo/redo and word-wrap layout.
//!
//! Pure std (no graphics), so it can be unit-tested on its own.

use std::borrow::Cow;
use std::time::{Duration, Instant};

const UNDO_LIMIT: usize = 200;
/// Typing or deleting within this gap of the previous edit is one undo step.
const RUN_GAP: Duration = Duration::from_millis(1000);

#[derive(Clone)]
struct Snapshot {
    text: Vec<char>,
    cursor: usize,
    anchor: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Typing,
    Deleting,
    Other,
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

/// What to draw: the text with any IME pre-edit text inserted at the cursor.
pub struct Display<'a> {
    pub chars: Cow<'a, [char]>,
    /// Cursor position in `chars`.
    pub cursor: usize,
    /// Range of the pre-edit text in `chars`, if any.
    pub preedit: Option<(usize, usize)>,
}

pub struct Editor {
    pub text: Vec<char>,
    pub cursor: usize,
    pub anchor: usize,
    /// Bumped whenever the saved text changes.
    pub text_version: u64,
    /// Bumped whenever the displayed text changes (text edits or pre-edit changes).
    pub display_version: u64,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    run: Option<(Kind, Instant)>,
    goal_x: Option<f32>,
    preedit: Vec<char>,
    preedit_cursor: usize,
}

impl Editor {
    /// A new editor with the cursor at the end of `text`.
    pub fn new(text: &str) -> Editor {
        let text: Vec<char> = text.chars().filter(|&c| c != '\r').collect();
        let end = text.len();
        Editor {
            text,
            cursor: end,
            anchor: end,
            text_version: 0,
            display_version: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            run: None,
            goal_x: None,
            preedit: Vec::new(),
            preedit_cursor: 0,
        }
    }

    pub fn text_string(&self) -> String {
        self.text.iter().collect()
    }

    /// The selected range, ordered, if anything is selected.
    pub fn selection(&self) -> Option<(usize, usize)> {
        if self.anchor == self.cursor {
            None
        } else {
            Some((self.anchor.min(self.cursor), self.anchor.max(self.cursor)))
        }
    }

    pub fn display(&self) -> Display<'_> {
        if self.preedit.is_empty() {
            return Display {
                chars: Cow::Borrowed(&self.text),
                cursor: self.cursor,
                preedit: None,
            };
        }
        let mut v = Vec::with_capacity(self.text.len() + self.preedit.len());
        v.extend_from_slice(&self.text[..self.cursor]);
        v.extend_from_slice(&self.preedit);
        v.extend_from_slice(&self.text[self.cursor..]);
        Display {
            chars: Cow::Owned(v),
            cursor: self.cursor + self.preedit_cursor.min(self.preedit.len()),
            preedit: Some((self.cursor, self.cursor + self.preedit.len())),
        }
    }

    // ---- IME pre-edit ----

    pub fn has_preedit(&self) -> bool {
        !self.preedit.is_empty()
    }

    /// Shows `text` at the cursor as uncommitted input; `cursor` is a char index inside it.
    pub fn set_preedit(&mut self, text: &str, cursor: usize) {
        if text.is_empty() {
            self.clear_preedit();
            return;
        }
        if self.selection().is_some() {
            self.checkpoint(Kind::Other);
            self.delete_selection_raw();
            self.text_version += 1;
        }
        self.preedit = text.chars().collect();
        self.preedit_cursor = cursor.min(self.preedit.len());
        self.display_version += 1;
    }

    pub fn clear_preedit(&mut self) {
        if !self.preedit.is_empty() {
            self.preedit.clear();
            self.preedit_cursor = 0;
            self.display_version += 1;
        }
    }

    // ---- editing ----

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            cursor: self.cursor,
            anchor: self.anchor,
        }
    }

    /// Records an undo step before an edit, merging runs of typing or deleting.
    fn checkpoint(&mut self, kind: Kind) {
        let now = Instant::now();
        let merge = kind != Kind::Other
            && matches!(self.run, Some((k, t)) if k == kind && now.duration_since(t) < RUN_GAP);
        if !merge {
            self.undo.push(self.snapshot());
            if self.undo.len() > UNDO_LIMIT {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.run = Some((kind, now));
        self.goal_x = None;
    }

    fn changed(&mut self) {
        self.text_version += 1;
        self.display_version += 1;
    }

    fn delete_selection_raw(&mut self) -> bool {
        match self.selection() {
            Some((a, b)) => {
                self.text.drain(a..b);
                self.cursor = a;
                self.anchor = a;
                true
            }
            None => false,
        }
    }

    /// Inserts text at the cursor, replacing the selection.
    pub fn insert(&mut self, s: &str) {
        let chars: Vec<char> = s.chars().filter(|&c| c != '\r').collect();
        if chars.is_empty() {
            return;
        }
        let kind = if chars.len() == 1 && chars[0] != '\n' {
            Kind::Typing
        } else {
            Kind::Other
        };
        self.checkpoint(kind);
        self.delete_selection_raw();
        let n = chars.len();
        self.text.splice(self.cursor..self.cursor, chars);
        self.cursor += n;
        self.anchor = self.cursor;
        self.changed();
    }

    pub fn backspace(&mut self) {
        if self.selection().is_none() && self.cursor == 0 {
            return;
        }
        self.checkpoint(Kind::Deleting);
        if !self.delete_selection_raw() {
            self.cursor -= 1;
            self.text.remove(self.cursor);
            self.anchor = self.cursor;
        }
        self.changed();
    }

    pub fn delete_forward(&mut self) {
        if self.selection().is_none() && self.cursor >= self.text.len() {
            return;
        }
        self.checkpoint(Kind::Deleting);
        if !self.delete_selection_raw() {
            self.text.remove(self.cursor);
            self.anchor = self.cursor;
        }
        self.changed();
    }

    pub fn backspace_word(&mut self) {
        if self.selection().is_some() {
            self.backspace();
            return;
        }
        let to = self.word_left_idx(self.cursor);
        if to == self.cursor {
            return;
        }
        self.checkpoint(Kind::Other);
        self.text.drain(to..self.cursor);
        self.cursor = to;
        self.anchor = to;
        self.changed();
    }

    pub fn delete_word_forward(&mut self) {
        if self.selection().is_some() {
            self.delete_forward();
            return;
        }
        let to = self.word_right_idx(self.cursor);
        if to == self.cursor {
            return;
        }
        self.checkpoint(Kind::Other);
        self.text.drain(self.cursor..to);
        self.anchor = self.cursor;
        self.changed();
    }

    // ---- undo / redo ----

    fn restore(&mut self, s: Snapshot) {
        self.text = s.text;
        self.cursor = s.cursor.min(self.text.len());
        self.anchor = s.anchor.min(self.text.len());
        self.run = None;
        self.goal_x = None;
        self.changed();
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

    // ---- movement ----

    pub fn move_to(&mut self, idx: usize, extend: bool) {
        self.cursor = idx.min(self.text.len());
        if !extend {
            self.anchor = self.cursor;
        }
        self.run = None;
        self.goal_x = None;
    }

    pub fn left(&mut self, extend: bool) {
        match self.selection() {
            Some((a, _)) if !extend => self.move_to(a, false),
            _ => self.move_to(self.cursor.saturating_sub(1), extend),
        }
    }

    pub fn right(&mut self, extend: bool) {
        match self.selection() {
            Some((_, b)) if !extend => self.move_to(b, false),
            _ => self.move_to(self.cursor + 1, extend),
        }
    }

    fn word_left_idx(&self, mut i: usize) -> usize {
        while i > 0 && class(self.text[i - 1]) == Class::Space {
            i -= 1;
        }
        if i > 0 {
            let c = class(self.text[i - 1]);
            while i > 0 && class(self.text[i - 1]) == c {
                i -= 1;
            }
        }
        i
    }

    fn word_right_idx(&self, mut i: usize) -> usize {
        let n = self.text.len();
        while i < n && class(self.text[i]) == Class::Space {
            i += 1;
        }
        if i < n {
            let c = class(self.text[i]);
            while i < n && class(self.text[i]) == c {
                i += 1;
            }
        }
        i
    }

    pub fn word_left(&mut self, extend: bool) {
        self.move_to(self.word_left_idx(self.cursor), extend);
    }

    pub fn word_right(&mut self, extend: bool) {
        self.move_to(self.word_right_idx(self.cursor), extend);
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.cursor = self.text.len();
        self.run = None;
        self.goal_x = None;
    }

    pub fn select_range(&mut self, a: usize, b: usize) {
        self.anchor = a.min(self.text.len());
        self.cursor = b.min(self.text.len());
        self.run = None;
        self.goal_x = None;
    }

    /// The word (run of same-class characters) around `idx`.
    pub fn word_range(&self, idx: usize) -> (usize, usize) {
        let n = self.text.len();
        if n == 0 {
            return (0, 0);
        }
        let i = idx.min(n - 1);
        let c = class(self.text[i]);
        let mut a = i;
        while a > 0 && class(self.text[a - 1]) == c {
            a -= 1;
        }
        let mut b = i + 1;
        while b < n && class(self.text[b]) == c {
            b += 1;
        }
        (a, b)
    }

    /// The paragraph (text between newlines) around `idx`, without the newline.
    pub fn paragraph_range(&self, idx: usize) -> (usize, usize) {
        let idx = idx.min(self.text.len());
        let start = self.text[..idx]
            .iter()
            .rposition(|&c| c == '\n')
            .map_or(0, |p| p + 1);
        let end = self.text[idx..]
            .iter()
            .position(|&c| c == '\n')
            .map_or(self.text.len(), |p| idx + p);
        (start, end)
    }

    /// Moves up or down by `delta` display lines, keeping the horizontal position.
    pub fn move_vertical(
        &mut self,
        layout: &Layout,
        delta: isize,
        extend: bool,
        adv: &mut dyn FnMut(char) -> f32,
    ) {
        let li = layout.line_of(self.cursor);
        let gx = match self.goal_x {
            Some(x) => x,
            None => layout.x_at(&self.text, li, self.cursor, adv),
        };
        let target = li as isize + delta;
        let idx = if target < 0 {
            0
        } else if target as usize >= layout.lines.len() {
            self.text.len()
        } else {
            layout.hit_x(&self.text, target as usize, gx, adv)
        };
        self.move_to(idx, extend);
        self.goal_x = Some(gx);
    }

    /// Moves to the start or end of the current display line.
    pub fn line_edge(&mut self, layout: &Layout, end: bool, extend: bool) {
        let l = &layout.lines[layout.line_of(self.cursor)];
        let idx = if !end {
            l.start
        } else if l.last {
            l.end
        } else {
            l.end.saturating_sub(1).max(l.start)
        };
        self.move_to(idx, extend);
    }
}

// ---- layout ----

/// One display line: chars `start..end` of the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub start: usize,
    pub end: usize,
    /// True for the last display line of a paragraph (the newline, if any, follows `end`).
    pub last: bool,
}

pub struct Layout {
    pub lines: Vec<Line>,
}

/// Characters that may be wrapped anywhere (no spaces between words).
fn wraps_anywhere(c: char) -> bool {
    matches!(c as u32,
        0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x2FFFF)
}

impl Layout {
    /// Word-wraps `text` to `max_width`, using `adv` for each character's width.
    pub fn build(text: &[char], max_width: f32, adv: &mut dyn FnMut(char) -> f32) -> Layout {
        let n = text.len();
        let mut lines = Vec::new();
        let mut i = 0;
        loop {
            let mut para_end = i;
            while para_end < n && text[para_end] != '\n' {
                para_end += 1;
            }
            let mut start = i;
            let mut x = 0.0f32;
            let mut last_break: Option<usize> = None;
            let mut j = i;
            while j < para_end {
                let c = text[j];
                let w = adv(c);
                if x + w > max_width && j > start {
                    let brk = match last_break {
                        Some(b) if b > start => b,
                        _ => j,
                    };
                    lines.push(Line {
                        start,
                        end: brk,
                        last: false,
                    });
                    start = brk;
                    x = 0.0;
                    last_break = None;
                    j = start;
                    continue;
                }
                x += w;
                if c == ' ' || wraps_anywhere(c) {
                    last_break = Some(j + 1);
                }
                j += 1;
            }
            lines.push(Line {
                start,
                end: para_end,
                last: true,
            });
            if para_end >= n {
                break;
            }
            i = para_end + 1;
        }
        Layout { lines }
    }

    /// The display line holding index `idx` (the last line starting at or before it).
    pub fn line_of(&self, idx: usize) -> usize {
        match self.lines.binary_search_by(|l| l.start.cmp(&idx)) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        }
    }

    /// Horizontal offset of `idx` within display line `line`.
    pub fn x_at(
        &self,
        text: &[char],
        line: usize,
        idx: usize,
        adv: &mut dyn FnMut(char) -> f32,
    ) -> f32 {
        let l = &self.lines[line];
        let to = idx.clamp(l.start, text.len());
        text[l.start..to].iter().map(|&c| adv(c)).sum()
    }

    /// The char index on display line `line` closest to horizontal offset `x`.
    pub fn hit_x(
        &self,
        text: &[char],
        line: usize,
        x: f32,
        adv: &mut dyn FnMut(char) -> f32,
    ) -> usize {
        let l = &self.lines[line];
        // On a wrapped line, the position after its last char is the start of the next line.
        let max_idx = if l.last {
            l.end
        } else {
            l.end.saturating_sub(1).max(l.start)
        };
        let mut pen = 0.0;
        for idx in l.start..max_idx {
            let w = adv(text[idx]);
            if x < pen + w / 2.0 {
                return idx;
            }
            pen += w;
        }
        max_idx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ed(s: &str) -> Editor {
        Editor::new(s)
    }

    fn ten(_: char) -> f32 {
        10.0
    }

    fn layout(text: &str, width: f32) -> (Vec<char>, Layout) {
        let chars: Vec<char> = text.chars().collect();
        let l = Layout::build(&chars, width, &mut ten);
        (chars, l)
    }

    fn ranges(l: &Layout) -> Vec<(usize, usize, bool)> {
        l.lines.iter().map(|l| (l.start, l.end, l.last)).collect()
    }

    #[test]
    fn insert_and_backspace() {
        let mut e = ed("");
        e.insert("a");
        e.insert("b");
        e.insert("é");
        assert_eq!(e.text_string(), "abé");
        e.backspace();
        assert_eq!(e.text_string(), "ab");
        e.move_to(0, false);
        e.backspace();
        assert_eq!(e.text_string(), "ab");
        e.delete_forward();
        assert_eq!(e.text_string(), "b");
    }

    #[test]
    fn new_strips_carriage_returns_and_puts_cursor_at_end() {
        let e = ed("a\r\nb");
        assert_eq!(e.text_string(), "a\nb");
        assert_eq!(e.cursor, 3);
    }

    #[test]
    fn selection_replace_and_delete() {
        let mut e = ed("hello world");
        e.select_range(0, 5);
        e.insert("bye");
        assert_eq!(e.text_string(), "bye world");
        e.select_range(3, 9);
        e.backspace();
        assert_eq!(e.text_string(), "bye");
        assert_eq!(e.cursor, 3);
        assert_eq!(e.selection(), None);
    }

    #[test]
    fn left_right_collapse_selection() {
        let mut e = ed("abcdef");
        e.select_range(4, 1);
        e.left(false);
        assert_eq!((e.cursor, e.selection()), (1, None));
        e.select_range(1, 4);
        e.right(false);
        assert_eq!((e.cursor, e.selection()), (4, None));
        e.right(true);
        assert_eq!(e.selection(), Some((4, 5)));
    }

    #[test]
    fn typing_is_one_undo_step_and_redo_works() {
        let mut e = ed("");
        for c in "hello".chars() {
            e.insert(&c.to_string());
        }
        e.insert("\n");
        e.insert("x");
        assert_eq!(e.text_string(), "hello\nx");
        assert!(e.undo()); // undoes "x"
        assert_eq!(e.text_string(), "hello\n");
        assert!(e.undo()); // undoes the newline
        assert_eq!(e.text_string(), "hello");
        assert!(e.undo()); // undoes the whole typed word
        assert_eq!(e.text_string(), "");
        assert!(!e.undo());
        assert!(e.redo());
        assert_eq!(e.text_string(), "hello");
        assert!(e.redo());
        assert!(e.redo());
        assert_eq!(e.text_string(), "hello\nx");
        assert!(!e.redo());
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut e = ed("");
        e.insert("a");
        e.undo();
        e.insert("b");
        assert!(!e.redo());
        assert_eq!(e.text_string(), "b");
    }

    #[test]
    fn versions_bump_on_edits() {
        let mut e = ed("a");
        let (t, d) = (e.text_version, e.display_version);
        e.move_to(0, false);
        assert_eq!((e.text_version, e.display_version), (t, d));
        e.insert("b");
        assert!(e.text_version > t && e.display_version > d);
        let (t, d) = (e.text_version, e.display_version);
        e.set_preedit("zhong", 5);
        assert_eq!(e.text_version, t);
        assert!(e.display_version > d);
    }

    #[test]
    fn word_movement_and_deletion() {
        let mut e = ed("foo bar, baz");
        e.word_left(false);
        assert_eq!(e.cursor, 9);
        e.word_left(false);
        assert_eq!(e.cursor, 8); // the comma
        e.word_left(false);
        assert_eq!(e.cursor, 4);
        e.word_right(false);
        assert_eq!(e.cursor, 7);
        e.move_to(12, false);
        e.backspace_word();
        assert_eq!(e.text_string(), "foo bar, ");
        e.move_to(0, false);
        e.delete_word_forward();
        assert_eq!(e.text_string(), " bar, ");
    }

    #[test]
    fn word_and_paragraph_ranges() {
        let e = ed("foo bar\nbaz qux");
        assert_eq!(e.word_range(5), (4, 7));
        assert_eq!(e.word_range(3), (3, 4));
        assert_eq!(e.paragraph_range(5), (0, 7));
        assert_eq!(e.paragraph_range(9), (8, 15));
        assert_eq!(e.paragraph_range(7), (0, 7));
        let cjk = ed("你好世界 ok");
        assert_eq!(cjk.word_range(1), (0, 4));
    }

    #[test]
    fn preedit_display() {
        let mut e = ed("ab");
        e.move_to(1, false);
        e.set_preedit("你好", 1);
        assert!(e.has_preedit());
        let d = e.display();
        assert_eq!(d.chars.iter().collect::<String>(), "a你好b");
        assert_eq!(d.cursor, 2);
        assert_eq!(d.preedit, Some((1, 3)));
        e.clear_preedit();
        let d = e.display();
        assert_eq!(d.chars.iter().collect::<String>(), "ab");
        assert_eq!(d.preedit, None);
    }

    #[test]
    fn layout_empty_and_trailing_newline() {
        let (_, l) = layout("", 100.0);
        assert_eq!(ranges(&l), vec![(0, 0, true)]);
        let (_, l) = layout("ab\n", 100.0);
        assert_eq!(ranges(&l), vec![(0, 2, true), (3, 3, true)]);
        let (_, l) = layout("a\n\nb", 100.0);
        assert_eq!(ranges(&l), vec![(0, 1, true), (2, 2, true), (3, 4, true)]);
    }

    #[test]
    fn layout_wraps_at_spaces() {
        // 10px per char, 50px wide = 5 chars per line.
        let (_, l) = layout("hello world", 50.0);
        assert_eq!(ranges(&l), vec![(0, 6, false), (6, 11, true)]);
        // A word longer than the line is split.
        let (_, l) = layout("abcdefghijkl", 50.0);
        assert_eq!(
            ranges(&l),
            vec![(0, 5, false), (5, 10, false), (10, 12, true)]
        );
    }

    #[test]
    fn layout_wraps_cjk_anywhere_and_always_progresses() {
        let (_, l) = layout("你好世界你好", 30.0);
        assert_eq!(ranges(&l), vec![(0, 3, false), (3, 6, true)]);
        // A line narrower than one char still places one char per line.
        let (_, l) = layout("abc", 1.0);
        assert_eq!(
            ranges(&l),
            vec![(0, 1, false), (1, 2, false), (2, 3, true)]
        );
    }

    #[test]
    fn line_of_and_x_at() {
        let (chars, l) = layout("hello world\nxy", 50.0);
        // lines: (0,6) (6,11) (12,14)
        assert_eq!(l.line_of(0), 0);
        assert_eq!(l.line_of(5), 0);
        assert_eq!(l.line_of(6), 1);
        assert_eq!(l.line_of(11), 1); // before the newline
        assert_eq!(l.line_of(12), 2);
        assert_eq!(l.line_of(14), 2);
        assert_eq!(l.x_at(&chars, 1, 9, &mut ten), 30.0);
        assert_eq!(l.x_at(&chars, 2, 14, &mut ten), 20.0);
    }

    #[test]
    fn hit_x_picks_nearest_boundary() {
        let (chars, l) = layout("hello world", 50.0);
        assert_eq!(l.hit_x(&chars, 0, 0.0, &mut ten), 0);
        assert_eq!(l.hit_x(&chars, 0, 4.9, &mut ten), 0);
        assert_eq!(l.hit_x(&chars, 0, 5.1, &mut ten), 1);
        assert_eq!(l.hit_x(&chars, 0, 26.0, &mut ten), 3);
        // Past the end of a wrapped line stays on that line (before its last char).
        assert_eq!(l.hit_x(&chars, 0, 999.0, &mut ten), 5);
        // Past the end of the last line goes to the end of the text.
        assert_eq!(l.hit_x(&chars, 1, 999.0, &mut ten), 11);
    }

    #[test]
    fn vertical_movement_keeps_column() {
        let text = "abcdef\nab\nabcdef";
        let mut e = ed(text);
        let (chars, l) = layout(text, 1000.0);
        assert_eq!(chars.len(), 16);
        e.move_to(5, false); // column 5 of line 0
        e.move_vertical(&l, 1, false, &mut ten);
        assert_eq!(e.cursor, 9); // clamped to end of "ab"
        e.move_vertical(&l, 1, false, &mut ten);
        assert_eq!(e.cursor, 15); // column 5 again
        e.move_vertical(&l, 1, false, &mut ten);
        assert_eq!(e.cursor, 16); // past the last line goes to the end
        e.move_vertical(&l, -3, false, &mut ten);
        assert_eq!(e.cursor, 0);
    }

    #[test]
    fn vertical_selection_extends() {
        let text = "abc\ndef";
        let mut e = ed(text);
        let (_, l) = layout(text, 1000.0);
        e.move_to(1, false);
        e.move_vertical(&l, 1, true, &mut ten);
        assert_eq!(e.selection(), Some((1, 5)));
    }

    #[test]
    fn line_edges_on_wrapped_lines() {
        let text = "hello world";
        let mut e = ed(text);
        let (_, l) = layout(text, 50.0); // (0,6) (6,11)
        e.move_to(2, false);
        e.line_edge(&l, true, false);
        assert_eq!(e.cursor, 5); // before the trailing space, still on line 0
        e.line_edge(&l, false, false);
        assert_eq!(e.cursor, 0);
        e.move_to(8, false);
        e.line_edge(&l, true, false);
        assert_eq!(e.cursor, 11);
        e.line_edge(&l, false, false);
        assert_eq!(e.cursor, 6);
    }

    #[test]
    fn select_all_and_insert() {
        let mut e = ed("abc");
        e.select_all();
        e.insert("x");
        assert_eq!(e.text_string(), "x");
    }
}
