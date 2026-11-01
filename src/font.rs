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
    collections::{HashMap, HashSet},
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
    pub h: u32,
    pub cov: Vec<u8>,
}

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
    adv: HashMap<char, f32>,
    glyphs: HashMap<char, Option<Bitmap>>,
    face_of: HashMap<char, usize>,
    uncovered: HashSet<char>,
}

/// Asks fontconfig for the best font file for a pattern.
fn fc_match(pattern: &str) -> Option<(PathBuf, u32)> {
    let out = Command::new("fc-match")
        .args(["-f", "%{file}\n%{index}\n", pattern])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let mut lines = s.lines();
    let file = lines.next()?.trim();
    if file.is_empty() {
        return None;
    }
    // The upper 16 bits can select a named instance of a variable font; we only need the face.
    let index = lines
        .next()
        .and_then(|l| l.trim().parse::<u32>().ok())
        .unwrap_or(0)
        & 0xFFFF;
    Some((PathBuf::from(file), index))
}

/// Memory-maps a font file. The mapping is kept for the life of the process.
fn load_face(path: &Path, index: u32) -> Option<FontRef<'static>> {
    let file = File::open(path).ok()?;
    // SAFETY: the mapping is read-only; a font file changing underneath us could only
    // produce garbled glyphs, as with any application that reads font files.
    let map = unsafe { Mmap::map(&file) }.ok()?;
    let map: &'static Mmap = Box::leak(Box::new(map));
    let data: &'static [u8] = &map[..];
    FontRef::try_from_slice_and_index(data, index).ok()
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
        let (face, key, fallback_pattern) = if spec.starts_with('/') {
            let path = PathBuf::from(spec);
            let face =
                load_face(&path, 0).ok_or_else(|| format!("could not load font file {spec}"))?;
            (face, (path, 0), "sans-serif".to_string())
        } else {
            let pattern = if spec.is_empty() { "sans-serif" } else { spec };
            let (path, index) = fc_match(pattern)
                .or_else(|| {
                    FALLBACK_FILES
                        .iter()
                        .map(PathBuf::from)
                        .find(|p| p.exists())
                        .map(|p| (p, 0))
                })
                .ok_or_else(|| {
                    "no font found: install fontconfig and a font, or set HYPRSTICKY_FONT to a \
                     font file path"
                        .to_string()
                })?;
            let face = load_face(&path, index).ok_or_else(|| {
                format!(
                    "could not load font {} (only TrueType/OpenType fonts are supported; \
                     set HYPRSTICKY_FONT to one)",
                    path.display()
                )
            })?;
            (face, (path, index), pattern.to_string())
        };
        let mut fonts = Fonts {
            faces: vec![face],
            loaded: vec![key],
            fallback_pattern,
            px: 0.0,
            line_height: 0.0,
            baseline: 0.0,
            space_width: 0.0,
            adv: HashMap::new(),
            glyphs: HashMap::new(),
            face_of: HashMap::new(),
            uncovered: HashSet::new(),
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
        self.adv.clear();
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
        if found.is_none() && !c.is_control() && !self.uncovered.contains(&c) {
            let pattern = format!("{}:charset={:x}", self.fallback_pattern, c as u32);
            if let Some((path, index)) = fc_match(&pattern) {
                if !self.loaded.iter().any(|(p, i)| *p == path && *i == index) {
                    if let Some(face) = load_face(&path, index) {
                        self.faces.push(face);
                    }
                    self.loaded.push((path, index));
                    found = covers(&self.faces);
                }
            }
            if found.is_none() {
                self.uncovered.insert(c);
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
        if let Some(&a) = self.adv.get(&c) {
            return a;
        }
        let i = self.face_index(c);
        let face = &self.faces[i];
        let scaled = face.as_scaled(pxscale(face, self.px));
        let a = scaled.h_advance(face.glyph_id(c));
        self.adv.insert(c, a);
        a
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
        self.glyphs.get(&c).and_then(|b| b.as_ref())
    }

    fn rasterize(&mut self, c: char) -> Option<Bitmap> {
        let i = self.face_index(c);
        let face = &self.faces[i];
        let glyph = face
            .glyph_id(c)
            .with_scale_and_position(pxscale(face, self.px), point(0.0, 0.0));
        let outlined = face.outline_glyph(glyph)?;
        let bounds = outlined.px_bounds();
        let w = bounds.width().ceil() as u32;
        let h = bounds.height().ceil() as u32;
        if w == 0 || h == 0 {
            return None;
        }
        let mut cov = vec![0u8; (w * h) as usize];
        outlined.draw(|x, y, v| {
            let idx = (y * w + x) as usize;
            if x < w && idx < cov.len() {
                cov[idx] = (v.clamp(0.0, 1.0) * 255.0) as u8;
            }
        });
        Some(Bitmap {
            left: bounds.min.x.floor() as i32,
            top: bounds.min.y.floor() as i32,
            w,
            h,
            cov,
        })
    }
}
