//! Word-wrapping and hit-testing for a block of text. No dependencies: character widths come
//! from a callback, so the font code stays separate.

/// One visual (wrapped) line.
pub struct Line {
    /// Index of the first char on the line.
    pub start: usize,
    /// Index just past the last char on the line (a newline is not part of any line).
    pub end: usize,
    /// `xs[i]` is the x offset of the char at `start + i`; `xs[end - start]` is the line width.
    pub xs: Vec<f32>,
}

pub struct Layout {
    pub lines: Vec<Line>,
}

/// Can a line break after this character? After spaces, and after CJK characters, which
/// have no spaces between words.
fn break_after(c: char) -> bool {
    c == ' ' || c == '\t' || (c as u32) >= 0x2E80
}

fn wrap_paragraph(
    chars: &[char],
    s: usize,
    e: usize,
    max_w: f32,
    adv: &mut dyn FnMut(char) -> f32,
    lines: &mut Vec<Line>,
) {
    if s == e {
        lines.push(Line {
            start: s,
            end: e,
            xs: vec![0.0],
        });
        return;
    }
    let mut start = s;
    let mut xs = vec![0.0f32];
    let mut x = 0.0f32;
    let mut last_break: Option<usize> = None;
    let mut i = s;
    while i < e {
        let w = adv(chars[i]);
        if x + w > max_w && i > start {
            // The char does not fit: break at the last break opportunity, or right here.
            let brk = match last_break {
                Some(b) if b > start => b,
                _ => i,
            };
            xs.truncate(brk - start + 1);
            lines.push(Line {
                start,
                end: brk,
                xs: std::mem::take(&mut xs),
            });
            start = brk;
            xs = vec![0.0];
            x = 0.0;
            last_break = None;
            i = start;
            continue;
        }
        x += w;
        xs.push(x);
        i += 1;
        if break_after(chars[i - 1]) {
            last_break = Some(i);
        }
    }
    lines.push(Line { start, end: e, xs });
}

impl Layout {
    pub fn build(chars: &[char], max_width: f32, adv: &mut dyn FnMut(char) -> f32) -> Layout {
        let n = chars.len();
        let mut lines = Vec::new();
        let mut i = 0;
        loop {
            let mut p_end = i;
            while p_end < n && chars[p_end] != '\n' {
                p_end += 1;
            }
            wrap_paragraph(chars, i, p_end, max_width, adv, &mut lines);
            if p_end >= n {
                break;
            }
            i = p_end + 1;
        }
        Layout { lines }
    }

    /// Index just past the last char of the text.
    pub fn text_end(&self) -> usize {
        self.lines.last().map_or(0, |l| l.end)
    }

    /// (line, column) of a cursor position. A position at a wrap point belongs to the line
    /// that starts there.
    pub fn locate(&self, idx: usize) -> (usize, usize) {
        let p = self.lines.partition_point(|l| l.start <= idx);
        let l = p.saturating_sub(1);
        let line = &self.lines[l];
        (
            l,
            (idx.saturating_sub(line.start)).min(line.end - line.start),
        )
    }

    pub fn x_of(&self, idx: usize) -> f32 {
        let (l, c) = self.locate(idx);
        self.lines[l].xs[c]
    }

    /// True if line `l` was wrapped, so the next line continues the same paragraph.
    pub fn is_wrapped(&self, l: usize) -> bool {
        l + 1 < self.lines.len() && self.lines[l + 1].start == self.lines[l].end
    }

    /// Last column the cursor may occupy on line `l` (not the wrap point, which belongs to the
    /// next line).
    fn max_col(&self, l: usize) -> usize {
        let len = self.lines[l].end - self.lines[l].start;
        if self.is_wrapped(l) && len > 0 {
            len - 1
        } else {
            len
        }
    }

    /// The cursor position on line `l` closest to x offset `x`.
    pub fn idx_at(&self, l: usize, x: f32) -> usize {
        let l = l.min(self.lines.len() - 1);
        let line = &self.lines[l];
        let mut best = 0;
        let mut best_d = f32::MAX;
        for col in 0..=self.max_col(l) {
            let d = (line.xs[col] - x).abs();
            if d < best_d {
                best = col;
                best_d = d;
            } else {
                break;
            }
        }
        line.start + best
    }

    pub fn line_start_idx(&self, idx: usize) -> usize {
        self.lines[self.locate(idx).0].start
    }

    pub fn line_end_idx(&self, idx: usize) -> usize {
        let l = self.locate(idx).0;
        self.lines[l].start + self.max_col(l)
    }

    /// Cursor position one line up, keeping the horizontal position `goal_x`.
    pub fn up(&self, idx: usize, goal_x: f32) -> usize {
        let l = self.locate(idx).0;
        if l == 0 {
            0
        } else {
            self.idx_at(l - 1, goal_x)
        }
    }

    /// Cursor position one line down, keeping the horizontal position `goal_x`.
    pub fn down(&self, idx: usize, goal_x: f32) -> usize {
        let l = self.locate(idx).0;
        if l + 1 >= self.lines.len() {
            self.text_end()
        } else {
            self.idx_at(l + 1, goal_x)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every char is 10 wide.
    fn build(text: &str, width: f32) -> (Vec<char>, Layout) {
        let chars: Vec<char> = text.chars().collect();
        let layout = Layout::build(&chars, width, &mut |_| 10.0);
        (chars, layout)
    }

    fn spans(l: &Layout) -> Vec<(usize, usize)> {
        l.lines.iter().map(|l| (l.start, l.end)).collect()
    }

    #[test]
    fn empty_text_has_one_empty_line() {
        let (_, l) = build("", 100.0);
        assert_eq!(spans(&l), vec![(0, 0)]);
        assert_eq!(l.locate(0), (0, 0));
        assert_eq!(l.x_of(0), 0.0);
    }

    #[test]
    fn newlines_make_lines_and_trailing_newline_makes_empty_line() {
        let (_, l) = build("ab\ncd\n", 100.0);
        assert_eq!(spans(&l), vec![(0, 2), (3, 5), (6, 6)]);
        assert_eq!(l.locate(2), (0, 2));
        assert_eq!(l.locate(3), (1, 0));
        assert_eq!(l.locate(6), (2, 0));
        assert_eq!(l.text_end(), 6);
    }

    #[test]
    fn wraps_at_spaces() {
        // width 100 = 10 chars per line
        let (_, l) = build("hello world again", 100.0);
        assert_eq!(spans(&l), vec![(0, 6), (6, 12), (12, 17)]);
    }

    #[test]
    fn long_word_breaks_mid_word() {
        let (_, l) = build("abcdefghijklmnopqrstuvwxyz", 100.0);
        assert_eq!(spans(&l), vec![(0, 10), (10, 20), (20, 26)]);
    }

    #[test]
    fn cjk_breaks_anywhere() {
        let (_, l) = build("圍道圍道圍道圍道圍道圍道", 100.0);
        assert_eq!(l.lines.len(), 2);
        assert_eq!(l.lines[0].end - l.lines[0].start, 10);
    }

    #[test]
    fn every_char_is_on_exactly_one_line() {
        let (chars, l) = build("the quick brown fox\n\njumps over the lazy dog ok", 80.0);
        let mut next = 0;
        for line in &l.lines {
            assert_eq!(line.start, next);
            next = line.end;
            if next < chars.len() && chars[next] == '\n' {
                next += 1;
            }
        }
        assert_eq!(next, chars.len());
    }

    #[test]
    fn single_char_wider_than_width_still_progresses() {
        let chars: Vec<char> = "abc".chars().collect();
        let l = Layout::build(&chars, 5.0, &mut |_| 10.0);
        assert_eq!(spans(&l), vec![(0, 1), (1, 2), (2, 3)]);
    }

    #[test]
    fn wrap_point_belongs_to_next_line() {
        let (_, l) = build("hello world", 100.0);
        assert_eq!(spans(&l), vec![(0, 6), (6, 11)]);
        assert_eq!(l.locate(6), (1, 0));
        assert_eq!(l.locate(5), (0, 5));
        assert!(l.is_wrapped(0));
        assert!(!l.is_wrapped(1));
    }

    #[test]
    fn x_positions() {
        let (_, l) = build("abc\nde", 100.0);
        assert_eq!(l.x_of(0), 0.0);
        assert_eq!(l.x_of(2), 20.0);
        assert_eq!(l.x_of(3), 30.0);
        assert_eq!(l.x_of(4), 0.0);
        assert_eq!(l.x_of(6), 20.0);
    }

    #[test]
    fn hit_testing_picks_nearest_boundary() {
        let (_, l) = build("abcd", 100.0);
        assert_eq!(l.idx_at(0, -50.0), 0);
        assert_eq!(l.idx_at(0, 4.0), 0);
        assert_eq!(l.idx_at(0, 6.0), 1);
        assert_eq!(l.idx_at(0, 24.0), 2);
        assert_eq!(l.idx_at(0, 1000.0), 4);
        assert_eq!(l.idx_at(99, 1000.0), 4);
    }

    #[test]
    fn hit_testing_never_lands_on_wrap_point_of_wrapped_line() {
        let (_, l) = build("hello world", 100.0);
        // Far right of the first line must stay on line 0, not jump to line 1's start.
        assert_eq!(l.idx_at(0, 1000.0), 5);
        assert_eq!(l.locate(l.idx_at(0, 1000.0)).0, 0);
    }

    #[test]
    fn vertical_movement_keeps_goal_x() {
        let (_, l) = build("abcdef\nab\nabcdef", 100.0);
        // From the end of line 0, down goes to the end of the short line, down again returns.
        let a = l.down(6, 60.0);
        assert_eq!(a, 9);
        let b = l.down(a, 60.0);
        assert_eq!(b, 16);
        assert_eq!(l.up(b, 60.0), 9);
        assert_eq!(l.up(a, 60.0), 6);
    }

    #[test]
    fn up_on_first_line_goes_to_start_and_down_on_last_to_end() {
        let (_, l) = build("abc\ndef", 100.0);
        assert_eq!(l.up(2, 20.0), 0);
        assert_eq!(l.down(5, 20.0), 7);
    }

    #[test]
    fn home_and_end() {
        let (_, l) = build("hello world\nxy", 100.0);
        // line 0 wraps into (0,6) and (6,11)
        assert_eq!(l.line_start_idx(8), 6);
        assert_eq!(l.line_end_idx(8), 11);
        assert_eq!(l.line_start_idx(2), 0);
        assert_eq!(l.line_end_idx(2), 5);
        assert_eq!(l.line_end_idx(13), 14);
    }
}
