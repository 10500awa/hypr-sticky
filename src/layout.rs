//! Word-wrapping and hit-testing for a block of markdown text.

use crate::{
    font::Fonts,
    md::{self, Style},
};

/// One visual (wrapped) line.
#[derive(Default)]
pub struct Line {
    /// Index of the first char on the line.
    pub start: usize,
    /// Index just past the last char on the line (a newline is not part of any line).
    pub end: usize,
    /// `xs[i]` is the x offset of the char at `start + i`; `xs[end - start]` is the line width.
    pub xs: Vec<f32>,
    /// Top of the line and its height; headings make a line taller.
    pub y: f32,
    pub h: f32,
    /// Style of the first char, which sets the line's height and baseline.
    pub style: Style,
}

#[derive(Default)]
pub struct Layout {
    pub lines: Vec<Line>,
    /// Markdown style of every char.
    pub styles: Vec<Style>,
}

/// Can a line break after this character? After spaces, and after CJK characters, which
/// have no spaces between words.
fn break_after(c: char) -> bool {
    c == ' ' || c == '\t' || (c as u32) >= 0x2E80
}

fn wrap_paragraph(
    chars: &[char], styles: &[Style], (s, e): (usize, usize), max_w: f32, fonts: &mut Fonts,
    lines: &mut Vec<Line>,
) {
    let mut start = s;
    let mut xs = vec![0.0f32];
    let mut x = 0.0f32;
    let mut last_break: Option<usize> = None;
    let mut i = s;
    while i < e {
        let w = fonts.advance(chars[i], styles[i]);
        if x + w > max_w && i > start {
            let brk = match last_break {
                Some(b) if b > start => b,
                _ => i,
            };
            xs.truncate(brk - start + 1);
            lines.push(Line { start, end: brk, xs: std::mem::take(&mut xs), ..Default::default() });
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
    lines.push(Line { start, end: e, xs, ..Default::default() });
}

impl Layout {
    pub fn build(chars: &[char], max_width: f32, fonts: &mut Fonts) -> Layout {
        let styles = md::styles(chars);
        let mut lines = Vec::new();
        let mut i = 0;
        loop {
            let p_end = chars[i..].iter().position(|&c| c == '\n').map_or(chars.len(), |p| i + p);
            wrap_paragraph(chars, &styles, (i, p_end), max_width, fonts, &mut lines);
            if p_end >= chars.len() {
                break;
            }
            i = p_end + 1;
        }
        let mut y = 0.0;
        for line in &mut lines {
            line.style = styles.get(line.start).copied().unwrap_or_default();
            (line.y, line.h) = (y, fonts.metrics(line.style).0);
            y += line.h;
        }
        Layout { lines, styles }
    }

    /// Total height of the text.
    pub fn height(&self) -> f32 {
        self.lines.last().map_or(0.0, |l| l.y + l.h)
    }

    /// The line at vertical offset `y` (the first or last line when `y` is outside the text).
    pub fn line_at(&self, y: f32) -> usize {
        self.lines.partition_point(|l| l.y + l.h <= y).min(self.lines.len() - 1)
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
        (l, (idx.saturating_sub(line.start)).min(line.end - line.start))
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
        if self.is_wrapped(l) && len > 0 { len - 1 } else { len }
    }

    /// The cursor position on line `l` closest to x offset `x`.
    pub fn idx_at(&self, l: usize, x: f32) -> usize {
        let l = l.min(self.lines.len() - 1);
        let line = &self.lines[l];
        let col = (0..=self.max_col(l))
            .min_by(|&a, &b| (line.xs[a] - x).abs().total_cmp(&(line.xs[b] - x).abs()))
            .unwrap_or(0);
        line.start + col
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
        if l == 0 { 0 } else { self.idx_at(l - 1, goal_x) }
    }

    /// Cursor position one line down, keeping the horizontal position `goal_x`.
    pub fn down(&self, idx: usize, goal_x: f32) -> usize {
        let l = self.locate(idx).0;
        if l + 1 >= self.lines.len() { self.text_end() } else { self.idx_at(l + 1, goal_x) }
    }
}
