//! Font loading and glyph rasterizing, kept small on purpose:
//!
//! * Fonts are found with `fc-match` (fontconfig), so your system font settings apply and no
//!   font database is held in memory.
//! * Font files are memory-mapped, so only the parts actually used count towards RAM.
//! * Bold, italic and monospace faces are only loaded once a note uses them.
//! * Glyph outlines are read lazily (ab_glyph), one character at a time.
//! * When a face lacks a character (for example Chinese, Japanese or Korean), a font that has
//!   it is looked up and loaded on demand.

use crate::md::Style;
/// Used when fontconfig is not available.
const FALLBACK_FILES: &[&str] = &[
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
];

/// Heading sizes relative to body text, by heading level (0 = body text).
const HEADING_SCALE: [f32; 7] = [1.0, 1.6, 1.4, 1.2, 1.1, 1.0, 1.0];

/// A rasterized glyph. `left` and `top` are offsets from the pen position on the baseline
/// to the bitmap's top-left corner (`top` is negative above the baseline).
pub struct Bitmap {
    pub left: i32,
    pub top: i32,
    pub w: u32,
    pub cov: Vec<u8>,
}

#[derive(Default)]
pub struct Fonts {
    faces: Vec<FontRef<'static>>,
    /// Where each face came from, in the same order as `faces`.
    loaded: Vec<(PathBuf, u32)>,
    /// First face to try for each bold/italic/monospace combination (see `Style::look`).
    primary: HashMap<u8, usize>,
    /// fontconfig pattern for the main font, also used to pick fallbacks in a matching style.
    pattern: String,
    /// Body font size in pixels per em.
    px: f32,
    glyphs: HashMap<(char, u8), Option<Bitmap>>,
    face_of: HashMap<(char, u8), usize>,
}

/// Asks fontconfig for the best font file for a pattern.
fn fc_match(pattern: &str) -> Option<(PathBuf, u32)> {
    let out = Command::new("fc-match").args(["-f", "%{file}\n%{index}", pattern]).output().ok()?;
    let out = String::from_utf8(out.stdout).ok()?;
    let (file, index) = out.split_once('\n')?;
    // The upper 16 bits can select a named instance of a variable font; we only need the face.
    let index = index.trim().parse::<u32>().unwrap_or(0) & 0xFFFF;
    (!file.is_empty()).then(|| (PathBuf::from(file), index))
}

/// Memory-maps a font file. The mapping is kept for the life of the process.
fn load_face(path: &Path, index: u32) -> Option<FontRef<'static>> {
    let file = File::open(path).ok()?;
    let map = unsafe { Mmap::map(&file) }.ok()?;
    FontRef::try_from_slice_and_index(Box::leak(Box::new(map)), index).ok()
}

/// Scale such that one em is `em_px` pixels. (ab_glyph's PxScale is the ascent+descent height.)
fn pxscale(face: &FontRef<'static>, em_px: f32) -> PxScale {
    let upem = face.units_per_em().unwrap_or(1000.0).max(1.0);
    PxScale::from(em_px * face.height_unscaled() / upem)
}

impl Fonts {
    /// `spec` is a fontconfig pattern (e.g. "sans-serif", "monospace", "JetBrains Mono") or the
    /// path of a font file. Defaults to "sans-serif" when empty.
    pub fn new(spec: &str) -> Result<Fonts, String> {
        let spec = spec.trim();
        let (key, pattern) = if spec.starts_with('/') {
            ((PathBuf::from(spec), 0), "sans-serif")
        } else {
            let pattern = if spec.is_empty() { "sans-serif" } else { spec };
            let found = fc_match(pattern).or_else(|| {
                FALLBACK_FILES.iter().map(PathBuf::from).find(|p| p.exists()).map(|p| (p, 0))
            });
            let msg = "no font found: install fontconfig and a font, or set HYPRSTICKY_FONT to a font file path";
            (found.ok_or(msg)?, pattern)
        };
        let mut fonts = Fonts { pattern: pattern.to_string(), ..Default::default() };
        fonts.add_face(key.clone()).ok_or_else(|| {
            let hint = "only TrueType/OpenType fonts work; set HYPRSTICKY_FONT to one";
            format!("could not load font {} ({hint})", key.0.display())
        })?;
        Ok(fonts)
    }

    /// Sets the body font size (pixels per em); clears the glyph cache if it changed.
    pub fn set_px(&mut self, px: f32) {
        if px != self.px {
            self.px = px;
            self.glyphs.clear();
        }
    }

    /// Font size of a style in pixels per em.
    fn size(&self, st: Style) -> f32 {
        self.px * HEADING_SCALE[st.level as usize]
    }

    /// (line height, baseline) in pixels for the size of a style.
    pub fn metrics(&self, st: Style) -> (f32, f32) {
        let (px, base) = (self.size(st), &self.faces[0]);
        let scaled = base.as_scaled(pxscale(base, px));
        let (ascent, descent) = (scaled.ascent(), scaled.descent());
        let line_height = (ascent - descent + scaled.line_gap().max(0.0)).max(px * 1.2).ceil();
        (line_height, ((line_height - (ascent - descent)) / 2.0 + ascent).round())
    }

    /// Index of the face at `key`, loading it if it is new.
    fn add_face(&mut self, key: (PathBuf, u32)) -> Option<usize> {
        if let Some(i) = self.loaded.iter().position(|k| *k == key) {
            return Some(i);
        }
        self.faces.push(load_face(&key.0, key.1)?);
        self.loaded.push(key);
        Some(self.faces.len() - 1)
    }

    /// The face to try first for a bold (1), italic (2) and monospace (4) combination.
    fn primary_face(&mut self, bits: u8) -> usize {
        if let Some(&i) = self.primary.get(&bits) {
            return i;
        }
        let mut pattern = if bits & 4 != 0 { "monospace".into() } else { self.pattern.clone() };
        pattern += if bits & 1 != 0 { ":bold" } else { "" };
        pattern += if bits & 2 != 0 { ":italic" } else { "" };
        let found =
            if bits == 0 { None } else { fc_match(&pattern).and_then(|k| self.add_face(k)) };
        let i = found.unwrap_or(0);
        self.primary.insert(bits, i);
        i
    }

    /// Index of the face to use for `c`, loading a fallback font if needed.
    fn face_index(&mut self, c: char, look: u8) -> usize {
        let bits = look & 7;
        if let Some(&i) = self.face_of.get(&(c, bits)) {
            return i;
        }
        let first = self.primary_face(bits);
        let covers = |f: &FontRef<'static>| f.glyph_id(c).0 != 0;
        let mut found = Some(first)
            .filter(|&i| covers(&self.faces[i]))
            .or_else(|| self.faces.iter().position(covers));
        if found.is_none() && !c.is_control() {
            let pattern = format!("{}:charset={:x}", self.pattern, c as u32);
            if fc_match(&pattern).and_then(|key| self.add_face(key)).is_some() {
                found = self.faces.iter().position(covers);
            }
        }
        let i = found.unwrap_or(first);
        self.face_of.insert((c, bits), i);
        i
    }

    /// Width of a character in pixels.
    pub fn advance(&mut self, c: char, st: Style) -> f32 {
        if c == '\t' {
            return 4.0 * self.advance(' ', st);
        }
        if c.is_control() {
            return 0.0;
        }
        let i = self.face_index(c, st.look());
        let face = &self.faces[i];
        face.as_scaled(pxscale(face, self.size(st))).h_advance(face.glyph_id(c))
    }

    /// The rasterized glyph for `c`, or None for characters with nothing to draw.
    pub fn glyph(&mut self, c: char, st: Style) -> Option<&Bitmap> {
        if c.is_control() || c == ' ' {
            return None;
        }
        let key = (c, st.look());
        if !self.glyphs.contains_key(&key) {
            let bitmap = self.rasterize(c, st);
            self.glyphs.insert(key, bitmap);
        }
        self.glyphs[&key].as_ref()
    }

    fn rasterize(&mut self, c: char, st: Style) -> Option<Bitmap> {
        let i = self.face_index(c, st.look());
        let face = &self.faces[i];
        let glyph =
            face.glyph_id(c).with_scale_and_position(pxscale(face, self.size(st)), point(0.0, 0.0));
        let outlined = face.outline_glyph(glyph)?;
        let bounds = outlined.px_bounds();
        let (w, h) = (bounds.width().ceil() as u32, bounds.height().ceil() as u32);
        if w == 0 || h == 0 {
            return None;
        }
        let mut cov = vec![0u8; (w * h) as usize];
        outlined.draw(|x, y, v| cov[(y * w + x) as usize] = (v.clamp(0.0, 1.0) * 255.0) as u8);
        Some(Bitmap { left: bounds.min.x.floor() as i32, top: bounds.min.y.floor() as i32, w, cov })
    }
}
