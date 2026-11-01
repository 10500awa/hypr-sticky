//! Font loading and glyph rasterizing, kept small on purpose:
//!
//! * Fonts are found with `fc-match` (fontconfig), so your system font settings apply and no
//!   font database is held in memory.
//! * Font files are memory-mapped, so only the parts actually used count towards RAM.
//! * Glyph outlines are read lazily (ab_glyph), one character at a time.
//! * When the main font lacks a character (for example Chinese, Japanese or Korean), a font
//!   that has it is looked up and loaded on demand.

use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use memmap2::Mmap;
use std::{
    collections::HashMap,
    fs::File,
    path::{Path, PathBuf},
    process::Command,
};

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
    loaded: Vec<(PathBuf, u32)>,
    /// fontconfig pattern for the main font, also used to pick fallbacks in a matching style.
    fallback_pattern: String,
    /// Font size in pixels per em.
    px: f32,
    pub line_height: f32,
    pub baseline: f32,
    pub space_width: f32,
    glyphs: HashMap<char, Option<Bitmap>>,
    face_of: HashMap<char, usize>,
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
        let face = load_face(&key.0, key.1).ok_or_else(|| {
            let (path, hint) = (key.0.display(), "only TrueType/OpenType fonts work");
            format!("could not load font {path} ({hint}; set HYPRSTICKY_FONT to one)")
        })?;
        let mut fonts = Fonts {
            faces: vec![face],
            loaded: vec![key],
            fallback_pattern: pattern.to_string(),
            ..Default::default()
        };
        fonts.set_px(16.0);
        Ok(fonts)
    }

    /// Sets the font size (pixels per em) and recomputes metrics; clears the caches if changed.
    pub fn set_px(&mut self, px: f32) {
        if (px - self.px).abs() < f32::EPSILON {
            return;
        }
        self.px = px;
        self.glyphs.clear();

        let base = &self.faces[0];
        let scaled = base.as_scaled(pxscale(base, px));
        let ascent = scaled.ascent();
        let descent = scaled.descent();
        let gap = scaled.line_gap().max(0.0);
        self.line_height = (ascent - descent + gap).max(px * 1.2).ceil();
        self.baseline = ((self.line_height - (ascent - descent)) / 2.0 + ascent).round();
        self.space_width = scaled.h_advance(base.glyph_id(' '));
    }

    /// Index of the face to use for `c`, loading a fallback font if needed.
    fn face_index(&mut self, c: char) -> usize {
        if let Some(&i) = self.face_of.get(&c) {
            return i;
        }
        let covers = |faces: &[FontRef<'static>]| faces.iter().position(|f| f.glyph_id(c).0 != 0);
        let mut found = covers(&self.faces);
        if found.is_none() && !c.is_control() {
            let pattern = format!("{}:charset={:x}", self.fallback_pattern, c as u32);
            if let Some(key) = fc_match(&pattern).filter(|k| !self.loaded.contains(k)) {
                self.faces.extend(load_face(&key.0, key.1));
                self.loaded.push(key);
                found = covers(&self.faces);
            }
        }
        let i = found.unwrap_or(0);
        self.face_of.insert(c, i);
        i
    }

    /// Width of a character in pixels.
    pub fn advance(&mut self, c: char) -> f32 {
        if c == '\t' {
            return 4.0 * self.space_width;
        }
        if c.is_control() {
            return 0.0;
        }
        let i = self.face_index(c);
        let face = &self.faces[i];
        face.as_scaled(pxscale(face, self.px)).h_advance(face.glyph_id(c))
    }

    /// The rasterized glyph for `c`, or None for characters with nothing to draw.
    pub fn glyph(&mut self, c: char) -> Option<&Bitmap> {
        if c.is_control() || c == ' ' {
            return None;
        }
        if !self.glyphs.contains_key(&c) {
            let bitmap = self.rasterize(c);
            self.glyphs.insert(c, bitmap);
        }
        self.glyphs[&c].as_ref()
    }

    fn rasterize(&mut self, c: char) -> Option<Bitmap> {
        let i = self.face_index(c);
        let face = &self.faces[i];
        let glyph =
            face.glyph_id(c).with_scale_and_position(pxscale(face, self.px), point(0.0, 0.0));
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
