//! Markdown styling. Every char of the note gets a `Style`; the text itself is never changed,
//! so editing works on the raw markdown and the markers (`**`, `#`, ...) stay visible, dimmed.
//! Supported: headings, bold, italic, strikethrough, inline and fenced code, links and images,
//! block quotes, lists and task lists, rules, and tables (shown in monospace).

#[derive(Clone, Copy, Default)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub mono: bool,
    pub strike: bool,
    /// A markup character, such as `**` or `#`.
    pub dim: bool,
    pub link: bool,
    pub quote: bool,
    /// Heading level 1 to 6, or 0 for body text.
    pub level: u8,
}

impl Style {
    /// Everything that changes the shape of a glyph: bold, italic, monospace and heading size.
    pub fn look(self) -> u8 {
        self.bold as u8 | (self.italic as u8) << 1 | (self.mono as u8) << 2 | self.level << 3
    }
}

const PAIRS: [(&str, fn(&mut Style)); 5] = [
    ("**", |s| s.bold = true),
    ("__", |s| s.bold = true),
    ("~~", |s| s.strike = true),
    ("*", |s| s.italic = true),
    ("_", |s| s.italic = true),
];

pub fn styles(chars: &[char]) -> Vec<Style> {
    let mut out = vec![Style::default(); chars.len()];
    let (mut a, mut fenced) = (0, false);
    while a <= chars.len() {
        let b = chars[a..].iter().position(|&c| c == '\n').map_or(chars.len(), |p| a + p);
        line(&chars[a..b], &mut out[a..b], &mut fenced);
        a = b + 1;
    }
    out
}

/// Does `c[i..b]` start with `t`?
fn at(c: &[char], i: usize, b: usize, t: &str) -> bool {
    i + t.len() <= b && t.chars().enumerate().all(|(k, ch)| c[i + k] == ch)
}

/// Styles one line; `fenced` tells whether we are inside a code block.
fn line(c: &[char], out: &mut [Style], fenced: &mut bool) {
    let n = c.len();
    let ind = c.iter().take_while(|x| x.is_whitespace()).count();
    let rest = &c[ind..];
    let mono = Style { mono: true, ..Style::default() };
    if at(c, ind, n, "```") || at(c, ind, n, "~~~") {
        *fenced = !*fenced;
        return out.fill(Style { dim: true, ..mono });
    }
    if *fenced || rest.first() == Some(&'|') {
        return out.fill(mono);
    }
    let first = rest.first().copied().unwrap_or(' ');
    if "-*_".contains(first) && rest.iter().filter(|&&x| x == first).count() >= 3 {
        if rest.iter().all(|&x| x == first || x == ' ') {
            return out.fill(Style { dim: true, ..Style::default() });
        }
    }

    let mut st = Style::default();
    let hashes = rest.iter().take_while(|&&x| x == '#').count();
    let digits = rest.iter().take_while(|x| x.is_ascii_digit()).count();
    let mut start = ind;
    if (1..=6).contains(&hashes) && rest.get(hashes) == Some(&' ') {
        st = Style { bold: true, level: hashes as u8, ..st };
        start += hashes + 1;
    } else if first == '>' {
        st.quote = true;
        start += 1 + (rest.get(1) == Some(&' ')) as usize;
    } else if (matches!(first, '-' | '*' | '+') && rest.get(1) == Some(&' '))
        || (digits > 0
            && matches!(rest.get(digits), Some('.' | ')'))
            && rest.get(digits + 1) == Some(&' '))
    {
        start += digits + 2;
        if at(c, start, n, "[ ] ") || at(c, start, n, "[x] ") || at(c, start, n, "[X] ") {
            start += 4;
        }
    }
    out[..start].fill(Style { dim: true, ..st });
    inline(c, start, n, st, out);
}

/// Styles `c[a..b]`: code, links, emphasis and strikethrough, nested to any depth.
fn inline(c: &[char], a: usize, b: usize, st: Style, out: &mut [Style]) {
    let mut i = a;
    while i < b {
        out[i] = st;
        i = span(c, i, b, st, out).unwrap_or(i + 1);
    }
}

/// If a span starts at `i`, styles it and returns the index just after it.
fn span(c: &[char], i: usize, b: usize, st: Style, out: &mut [Style]) -> Option<usize> {
    let dim = Style { dim: true, ..st };
    let find = |from: usize, ch: char| (from..b).find(|&j| c[j] == ch);
    match c[i] {
        '\\' if i + 1 < b => {
            out[i] = dim;
            out[i + 1] = st;
            return Some(i + 2);
        }
        '`' => {
            let j = find(i + 1, '`')?;
            out[i] = dim;
            out[j] = dim;
            out[i + 1..j].fill(Style { mono: true, ..st });
            return Some(j + 1);
        }
        '[' => {
            let j = find(i + 1, ']')?;
            let k = if j + 1 < b && c[j + 1] == '(' { find(j + 2, ')')? } else { return None };
            out[i] = dim;
            inline(c, i + 1, j, Style { link: true, ..st }, out);
            out[j..=k].fill(Style { dim: true, link: true, ..st });
            return Some(k + 1);
        }
        'h' if at(c, i, b, "http://") || at(c, i, b, "https://") => {
            let end = (i..b).find(|&j| c[j].is_whitespace()).unwrap_or(b);
            out[i..end].fill(Style { link: true, ..st });
            return Some(end);
        }
        _ => {}
    }
    for (tok, set) in PAIRS {
        let n = tok.len();
        let under = tok.starts_with('_');
        if !at(c, i, b, tok) || i + n >= b || c[i + n].is_whitespace() {
            continue;
        }
        if under && i > 0 && c[i - 1].is_alphanumeric() {
            continue;
        }
        // A closing marker ends a run of the same char and does not follow whitespace.
        let close = |j: usize| {
            let next = c.get(j + n).filter(|_| j + n < b);
            at(c, j, b, tok)
                && !c[j - 1].is_whitespace()
                && (n == 2 || c[j - 1] != c[i])
                && next != Some(&c[i])
                && !(under && next.is_some_and(|x| x.is_alphanumeric()))
        };
        if let Some(j) = (i + n + 1..b).find(|&j| close(j)) {
            let mut inner = st;
            set(&mut inner);
            out[i..i + n].fill(dim);
            out[j..j + n].fill(dim);
            inline(c, i + n, j, inner, out);
            return Some(j + n);
        }
    }
    None
}
