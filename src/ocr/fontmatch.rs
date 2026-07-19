// =============================================================================
// PDF Lks Text Editor - Éditeur PDF avec OCR et édition de texte
// Créé par Leksa667 (https://github.com/Leksa667)
//
// Ce logiciel est en libre service. Vous pouvez l'utiliser, le modifier
// et le distribuer librement, à condition de créditer l'auteur original.
// Aucune garantie n'est fournie.
// =============================================================================

use crate::ocr::tesseract::OcrWord;
use fontdue::Font;
use image::GrayImage;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Height every word bitmap is normalised to before comparison.
const NORM_HEIGHT: usize = 40;
const MAX_WORDS: usize = 8;
const MIN_WORD_LEN: usize = 4;
/// How far a candidate may slide to line up with the scan.
const SHIFT: i32 = 3;
/// Candidates kept after the cheap first pass.
const SHORTLIST: usize = 64;

#[derive(Clone, Debug)]
pub struct FontMatch {
    pub path: PathBuf,
    pub family: String,
    pub score: f32,
}

/// An ink-coverage map: 0 = paper, 1 = full ink. Grey rather than black-and-
/// white on purpose — at scanner resolution a word is barely 30 px tall, and
/// thresholding it away throws out exactly the stroke detail that separates two
/// typefaces.
#[derive(Clone)]
pub struct Sample {
    data: Vec<f32>,
    width: usize,
    /// Ink height of the source crop in original pixels, before normalisation.
    src_height: usize,
}

impl Sample {
    pub fn width(&self) -> usize {
        self.width
    }

    pub fn src_height(&self) -> usize {
        self.src_height
    }

    pub fn value(&self, x: usize, y: usize) -> f32 {
        self.at(x as i32, y as i32)
    }

    pub fn ink(&self) -> f32 {
        self.ink_impl()
    }

    fn at(&self, x: i32, y: i32) -> f32 {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= NORM_HEIGHT {
            return 0.0;
        }
        self.data[y as usize * self.width + x as usize]
    }

    fn ink_impl(&self) -> f32 {
        self.data.iter().sum::<f32>() / (self.width * NORM_HEIGHT).max(1) as f32
    }

    /// Area-averaged rescale of a coverage map to the comparison height.
    fn build(coverage: &[f32], width: usize, height: usize) -> Option<Sample> {
        // Tight crop on the ink.
        let (mut x0, mut y0, mut x1, mut y1) = (width, height, 0usize, 0usize);
        for y in 0..height {
            for x in 0..width {
                if coverage[y * width + x] > 0.35 {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x + 1);
                    y1 = y1.max(y + 1);
                }
            }
        }
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let (cw, ch) = (x1 - x0, y1 - y0);

        let out_h = NORM_HEIGHT;
        let out_w = ((cw as f32 / ch as f32) * out_h as f32)
            .round()
            .clamp(1.0, 1024.0) as usize;

        let mut data = vec![0.0f32; out_w * out_h];
        for y in 0..out_h {
            let sy0 = y0 + y * ch / out_h;
            let sy1 = (y0 + ((y + 1) * ch).div_ceil(out_h)).max(sy0 + 1).min(y1);
            for x in 0..out_w {
                let sx0 = x0 + x * cw / out_w;
                let sx1 = (x0 + ((x + 1) * cw).div_ceil(out_w)).max(sx0 + 1).min(x1);

                let mut sum = 0.0f32;
                let mut n = 0usize;
                for sy in sy0..sy1 {
                    for sx in sx0..sx1 {
                        sum += coverage[sy * width + sx];
                        n += 1;
                    }
                }
                data[y * out_w + x] = if n > 0 { sum / n as f32 } else { 0.0 };
            }
        }

        Some(Sample { data, width: out_w, src_height: ch })
    }

    fn peak(&self) -> f32 {
        self.data.iter().cloned().fold(0.0f32, f32::max)
    }

    /// Fraction of pixels that are ink at the canonical threshold. This is the
    /// weight signal: it distinguishes a regular from its bold.
    fn ink_fraction(&self) -> f32 {
        self.ink_fraction_at(0.45)
    }

    fn ink_fraction_at(&self, frac_of_peak: f32) -> f32 {
        let t = self.peak() * frac_of_peak;
        if t <= 0.0 {
            return 0.0;
        }
        self.data.iter().filter(|v| **v >= t).count() as f32 / self.data.len().max(1) as f32
    }

    /// Ink/paper map at the canonical threshold, blurred for correlation.
    fn binarized(&self) -> Sample {
        self.thresholded(self.peak() * 0.45)
    }

    /// Ink/paper map whose ink fraction is forced to `target`, blurred.
    ///
    /// Scanning and thresholding distort stroke weight in opposite directions:
    /// Otsu on blurred paper thins the scan, while binarising a crisp
    /// rasterisation fattens it. Comparing shapes at *equal* ink removes that
    /// bias, so the true face is no longer beaten by a thinner lookalike; the
    /// weight itself is judged separately by `ink_fraction`.
    ///
    /// The blur comes *before* the cut, mimicking print-and-scan: a hairline
    /// blurred below the threshold disappears exactly as it does on paper.
    /// Cutting the crisp map instead keeps every hairline and starves the
    /// stems, which is how a Times New Roman loses to a hairline lookalike.
    fn binarized_to_ink(&self, target: f32) -> Sample {
        let soft = self.blurred();
        let mut sorted: Vec<f32> = soft.data.clone();
        sorted.sort_by(|a, b| b.total_cmp(a));
        let idx = ((target * sorted.len() as f32) as usize).min(sorted.len().saturating_sub(1));
        let t = sorted[idx].max(soft.peak() * 0.10);
        soft.thresholded(t)
    }

    fn thresholded(&self, t: f32) -> Sample {
        let data = self
            .data
            .iter()
            .map(|v| if *v >= t && t > 0.0 { 1.0 } else { 0.0 })
            .collect();
        // Two blur passes: letters drift by a pixel or two inside the word
        // between the print-and-scan and the rasterised version, and a global
        // shift cannot line them all up at once. The correlation has to be
        // tolerant to that jitter or the true face loses to chance.
        Sample { data, width: self.width, src_height: self.src_height }
            .blurred()
            .blurred()
    }

    /// A light blur absorbs scanner noise and the stroke-weight gap between a
    /// clean rasterisation and inked paper, without erasing letter shapes.
    fn blurred(&self) -> Sample {
        let mut data = vec![0.0f32; self.width * NORM_HEIGHT];
        for y in 0..NORM_HEIGHT as i32 {
            for x in 0..self.width as i32 {
                let mut sum = 0.0;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let w = if dx == 0 && dy == 0 { 4.0 } else { 1.0 };
                        sum += w * self.at(x + dx, y + dy);
                    }
                }
                data[y as usize * self.width + x as usize] = sum / 12.0;
            }
        }
        Sample { data, width: self.width, src_height: self.src_height }
    }

    /// Ink per column, normalised. The horizontal rhythm of a word — where the
    /// stems and counters fall — is a strong signature of a face and survives
    /// scanner blur far better than the outline does.
    fn column_profile(&self) -> Vec<f32> {
        let mut profile = vec![0.0f32; self.width];
        for x in 0..self.width {
            for y in 0..NORM_HEIGHT {
                profile[x] += self.data[y * self.width + x];
            }
        }
        profile
    }

    /// Ink per row. Tight-cropping and rescaling erase absolute size, but the
    /// *vertical proportions* survive in this profile: a large x-height reads
    /// as a tall dense band, long ascenders as a thin sparse one. That is what
    /// separates a Verdana from an Arial once their widths are normalised away.
    fn row_profile(&self) -> Vec<f32> {
        let mut profile = vec![0.0f32; NORM_HEIGHT];
        for y in 0..NORM_HEIGHT {
            for x in 0..self.width {
                profile[y] += self.data[y * self.width + x];
            }
        }
        let w = self.width.max(1) as f32;
        for v in profile.iter_mut() {
            *v /= w;
        }
        profile
    }
}

/// Zero-mean normalised correlation of two row profiles, best over small shifts.
fn row_correlation(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    if n == 0 {
        return 0.0;
    }
    let get = |v: &[f32], i: i32| -> f32 {
        if i < 0 || i as usize >= v.len() { 0.0 } else { v[i as usize] }
    };
    let mut best = 0.0f32;
    for shift in -2..=2i32 {
        let (mut sa, mut sb) = (0.0f32, 0.0f32);
        for i in 0..n as i32 {
            sa += get(a, i);
            sb += get(b, i - shift);
        }
        let (ma, mb) = (sa / n as f32, sb / n as f32);
        let (mut num, mut da, mut db) = (0.0f32, 0.0f32, 0.0f32);
        for i in 0..n as i32 {
            let va = get(a, i) - ma;
            let vb = get(b, i - shift) - mb;
            num += va * vb;
            da += va * va;
            db += vb * vb;
        }
        if da > 0.0 && db > 0.0 {
            best = best.max(num / (da.sqrt() * db.sqrt()));
        }
    }
    best.clamp(0.0, 1.0)
}

/// Zero-mean normalised correlation of two column profiles, per band with a
/// band-local shift, for the same reason as `segmented_shape`.
fn profile_correlation(a: &[f32], b: &[f32]) -> f32 {
    const BANDS: i32 = 4;
    let width = a.len().max(b.len()) as i32;
    let get = |v: &[f32], i: i32| -> f32 {
        if i < 0 || i as usize >= v.len() { 0.0 } else { v[i as usize] }
    };

    let mut total = 0.0f32;
    let mut weight = 0.0f32;
    for band in 0..BANDS {
        let x0 = band * width / BANDS;
        let x1 = if band == BANDS - 1 { width } else { (band + 1) * width / BANDS };
        if x1 <= x0 {
            continue;
        }
        let mut best = 0.0f32;
        for shift in -SHIFT..=SHIFT {
            let (mut sa, mut sb) = (0.0f32, 0.0f32);
            for i in x0..x1 {
                sa += get(a, i);
                sb += get(b, i - shift);
            }
            let n = (x1 - x0) as f32;
            let (ma, mb) = (sa / n, sb / n);
            let (mut num, mut da, mut db) = (0.0f32, 0.0f32, 0.0f32);
            for i in x0..x1 {
                let va = get(a, i) - ma;
                let vb = get(b, i - shift) - mb;
                num += va * vb;
                da += va * va;
                db += vb * vb;
            }
            if da > 0.0 && db > 0.0 {
                best = best.max(num / (da.sqrt() * db.sqrt()));
            }
        }
        let w = (x1 - x0) as f32;
        total += best.clamp(0.0, 1.0) * w;
        weight += w;
    }
    if weight <= 0.0 {
        return 0.0;
    }
    total / weight
}

/// Zero-mean normalised cross-correlation of two coverage maps over a column
/// band, at a given shift.
fn correlate(a: &Sample, b: &Sample, x0: i32, x1: i32, dx: i32, dy: i32) -> f32 {
    let n = ((x1 - x0) * NORM_HEIGHT as i32) as f32;

    let (mut sa, mut sb) = (0.0f32, 0.0f32);
    for y in 0..NORM_HEIGHT as i32 {
        for x in x0..x1 {
            sa += a.at(x, y);
            sb += b.at(x - dx, y - dy);
        }
    }
    let (ma, mb) = (sa / n, sb / n);

    let (mut num, mut da, mut db) = (0.0f32, 0.0f32, 0.0f32);
    for y in 0..NORM_HEIGHT as i32 {
        for x in x0..x1 {
            let va = a.at(x, y) - ma;
            let vb = b.at(x - dx, y - dy) - mb;
            num += va * vb;
            da += va * va;
            db += vb * vb;
        }
    }
    if da <= 0.0 || db <= 0.0 {
        return 0.0;
    }
    num / (da.sqrt() * db.sqrt())
}

/// Shape correlation where each column band of the word aligns on its own.
///
/// Between inked paper and a rasterisation, letters drift by a pixel or two
/// *inside* the word — the drift accumulates along it, so one global shift can
/// never line up the whole word at once, and the true face scores barely above
/// its lookalikes. Letting each quarter find its own alignment absorbs the
/// drift while still comparing actual letterforms.
fn segmented_shape(a: &Sample, b: &Sample) -> f32 {
    const BANDS: i32 = 4;
    let width = a.width.max(b.width) as i32;
    let mut total = 0.0f32;
    let mut weight = 0.0f32;
    for band in 0..BANDS {
        let x0 = band * width / BANDS;
        let x1 = if band == BANDS - 1 { width } else { (band + 1) * width / BANDS };
        if x1 <= x0 {
            continue;
        }
        let mut best = 0.0f32;
        for dy in -2..=2 {
            for dx in -SHIFT..=SHIFT {
                best = best.max(correlate(a, b, x0, x1, dx, dy));
            }
        }
        let w = (x1 - x0) as f32;
        total += best.clamp(0.0, 1.0) * w;
        weight += w;
    }
    if weight <= 0.0 {
        return 0.0;
    }
    total / weight
}

/// How alike a scanned word and a rendered one are, in [0, 1].
///
/// Three cues, because outline overlap alone does not separate close typefaces:
/// the shape correlation, the aspect ratio (a word's width at a given height is
/// a signature of the face), and the ink density (which is what really tells a
/// regular from its own bold).
pub fn similarity(scanned: &Sample, rendered: &Sample) -> f32 {
    let width_ratio = scanned.width.min(rendered.width) as f32
        / scanned.width.max(rendered.width).max(1) as f32;

    // A word set in the wrong face comes out the wrong length: reject cheaply.
    if width_ratio < 0.7 {
        return 0.0;
    }

    let scan_bin = scanned.binarized();
    let rend_bin = rendered.binarized_to_ink(scanned.ink_fraction());

    let shape = segmented_shape(&scan_bin, &rend_bin);
    let profile = profile_correlation(&scan_bin.column_profile(), &rend_bin.column_profile());
    let rows = row_correlation(&scan_bin.row_profile(), &rend_bin.row_profile());

    // Printing and scanning shift stroke weight systematically: Otsu thins the
    // scanned strokes while binarising crisp anti-aliasing fattens the
    // rasterised ones, by roughly a third. The rendered ink is therefore read
    // at a higher cut so that equal weights actually compare as equal, and the
    // term still separates a regular from its own bold.
    let ia = scanned.ink_fraction_at(0.45);
    let ib = rendered.ink_fraction_at(0.70);
    let density = 1.0 - (ia - ib).abs() / ia.max(0.01);

    shape.powf(1.2)
        * profile.powf(0.8)
        * rows.powf(0.5)
        * width_ratio.powi(4)
        * (0.5 + 0.5 * density.clamp(0.0, 1.0))
}

/// Cheap first pass: no shift search, no density term.
fn quick_score(scan_bin: &Sample, scan_ink: f32, rendered: &Sample) -> f32 {
    let width_ratio = scan_bin.width.min(rendered.width) as f32
        / scan_bin.width.max(rendered.width).max(1) as f32;
    if width_ratio < 0.65 {
        return 0.0;
    }
    // The column profile with band-local alignment: cheap, and unlike a rigid
    // zero-shift correlation it cannot throw the true face out of the
    // shortlist over a pixel of letter drift.
    let rend_bin = rendered.binarized_to_ink(scan_ink);
    profile_correlation(&scan_bin.column_profile(), &rend_bin.column_profile()) * width_ratio
}

/// Otsu threshold of a crop, used only to normalise ink coverage.
fn otsu(values: &[u8]) -> u8 {
    let mut histogram = [0u32; 256];
    for v in values {
        histogram[*v as usize] += 1;
    }
    let total = values.len() as f32;
    let sum: f32 = (0..256).map(|i| i as f32 * histogram[i] as f32).sum();

    let (mut sum_b, mut weight_b, mut best, mut threshold) = (0.0f32, 0.0f32, -1.0f32, 128u8);
    for (t, count) in histogram.iter().enumerate() {
        weight_b += *count as f32;
        if weight_b == 0.0 {
            continue;
        }
        let weight_f = total - weight_b;
        if weight_f == 0.0 {
            break;
        }
        sum_b += t as f32 * *count as f32;
        let mean_b = sum_b / weight_b;
        let mean_f = (sum - sum_b) / weight_f;
        let variance = weight_b * weight_f * (mean_b - mean_f) * (mean_b - mean_f);
        if variance > best {
            best = variance;
            threshold = t as u8;
        }
    }
    threshold
}

/// The scanned ink of one OCR word, as a coverage map.
pub fn word_sample(image: &GrayImage, word: &OcrWord) -> Option<Sample> {
    let (x, y, w, h) = word.bbox;
    if w < 12.0 || h < 12.0 {
        return None;
    }
    let (x0, y0) = (x as u32, y as u32);
    let (x1, y1) = ((x + w) as u32, (y + h) as u32);
    let (cw, ch) = ((x1 - x0) as usize, (y1 - y0) as usize);
    if cw == 0 || ch == 0 || cw > 4000 || ch > 4000 {
        return None;
    }

    let mut values = Vec::with_capacity(cw * ch);
    for py in y0..y1 {
        for px in x0..x1 {
            let v = if px < image.width() && py < image.height() {
                image.get_pixel(px, py)[0]
            } else {
                255
            };
            values.push(v);
        }
    }

    // Ink coverage: paper (above the threshold) is 0, the darkest pixel is 1.
    let threshold = otsu(&values) as f32;
    let darkest = *values.iter().min().unwrap_or(&0) as f32;
    let span = (threshold - darkest).max(1.0);
    let coverage: Vec<f32> = values
        .iter()
        .map(|v| ((threshold - *v as f32) / span).clamp(0.0, 1.0))
        .collect();

    Sample::build(&coverage, cw, ch)
}

/// Rasterises `text` with `font` at roughly the scanned crop's own pixel height
/// and returns its coverage map. Rendering at the scan's size, rather than at a
/// comfortable large size, makes both maps lose detail to quantisation the same
/// way before they are compared.
fn render_sample(font: &Font, text: &str, target_ink_h: usize) -> Option<Sample> {
    let size = (target_ink_h as f32 * 1.35).clamp(18.0, 160.0);
    let mut glyphs = Vec::new();
    let mut pen = 0.0f32;
    let (mut top, mut bottom) = (f32::MAX, f32::MIN);

    for c in text.chars() {
        let (metrics, bitmap) = font.rasterize(c, size);
        let x = pen + metrics.xmin as f32;
        let y = -(metrics.ymin as f32 + metrics.height as f32);
        if metrics.width > 0 && metrics.height > 0 {
            top = top.min(y);
            bottom = bottom.max(y + metrics.height as f32);
            glyphs.push((x, y, metrics.width, metrics.height, bitmap));
        }
        pen += metrics.advance_width;
    }
    if glyphs.is_empty() || pen <= 0.0 {
        return None;
    }

    let width = (pen.ceil() as usize + 8).min(12000);
    let height = ((bottom - top).ceil() as usize + 8).min(3000);
    let mut coverage = vec![0.0f32; width * height];

    for (x, y, gw, gh, bitmap) in glyphs {
        let ox = x.round() as isize + 4;
        let oy = (y - top).round() as isize + 4;
        for gy in 0..gh {
            for gx in 0..gw {
                let px = ox + gx as isize;
                let py = oy + gy as isize;
                if px < 0 || py < 0 || px as usize >= width || py as usize >= height {
                    continue;
                }
                let v = bitmap[gy * gw + gx] as f32 / 255.0;
                let slot = &mut coverage[py as usize * width + px as usize];
                *slot = slot.max(v);
            }
        }
    }

    // Put the rasterisation through the same mill as the paper: print blur at
    // native size, then greyscale, then the identical Otsu-based coverage.
    // Whatever detail scanning destroys is then destroyed on both sides, and
    // the true face matches itself again instead of matching whichever
    // lookalike happens to resemble its own ruins.
    let mut printed = vec![0.0f32; width * height];
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let mut sum = 0.0f32;
            let mut wsum = 0.0f32;
            for dy in -1..=1i32 {
                for dx in -1..=1i32 {
                    let (sx, sy) = (x + dx, y + dy);
                    if sx >= 0 && sy >= 0 && (sx as usize) < width && (sy as usize) < height {
                        let w = if dx == 0 && dy == 0 { 2.0 } else { 1.0 };
                        sum += w * coverage[sy as usize * width + sx as usize];
                        wsum += w;
                    }
                }
            }
            printed[y as usize * width + x as usize] = sum / wsum;
        }
    }

    let gray: Vec<u8> = printed
        .iter()
        .map(|v| (235.0 - v * 205.0).clamp(0.0, 255.0) as u8)
        .collect();
    let threshold = otsu(&gray) as f32;
    let darkest = *gray.iter().min().unwrap_or(&0) as f32;
    let span = (threshold - darkest).max(1.0);
    let coverage: Vec<f32> = gray
        .iter()
        .map(|v| ((threshold - *v as f32) / span).clamp(0.0, 1.0))
        .collect();

    Sample::build(&coverage, width, height)
}

pub fn render_sample_debug(font: &Font, text: &str, target_ink_h: usize) -> Option<Sample> {
    render_sample(font, text, target_ink_h)
}

/// (shape, profile, width_ratio, density) — the four factors of `similarity`.
pub fn similarity_parts(scanned: &Sample, rendered: &Sample) -> (f32, f32, f32, f32) {
    let width_ratio = scanned.width.min(rendered.width) as f32
        / scanned.width.max(rendered.width).max(1) as f32;
    let scan_bin = scanned.binarized();
    let rend_bin = rendered.binarized_to_ink(scanned.ink_fraction());
    let shape = segmented_shape(&scan_bin, &rend_bin);
    let profile = profile_correlation(&scan_bin.column_profile(), &rend_bin.column_profile());
    let ia = scanned.ink_fraction_at(0.45);
    let ib = rendered.ink_fraction_at(0.70);
    let density = 1.0 - (ia - ib).abs() / ia.max(0.01);
    (shape, profile, width_ratio, density)
}

fn load_font(path: &Path) -> Option<Font> {
    let data = std::fs::read(path).ok()?;
    Font::from_bytes(data, fontdue::FontSettings::default()).ok()
}

/// Prior probability that a document is set in this face.
///
/// At scan resolution the pixel evidence saturates: Times New Roman and the
/// latin glyphs of Mongolian Baiti tie to the third decimal. What breaks the
/// tie in reality is that no French document is typeset in Mongolian Baiti.
/// Classic document faces keep their full score; fonts that exist to cover
/// non-latin scripts, symbols or UI corners need to *beat* them clearly.
pub fn usage_prior(family: &str) -> f32 {
    let f = family.to_ascii_lowercase();

    let script_or_symbol = [
        "ebrima", "gadugi", "javanese", "leelawadee", "malgun", "mongolian", "myanmar",
        "nirmala", "phagspa", "tai le", "tai lue", "yi baiti", "himalaya", "simsun", "mingliu",
        "ms gothic", "yu gothic", "meiryo", "sylfaen", "sans serif collection", "webdings",
        "wingdings", "marlett", "symbol", "emoji", "historic", "javanese text", "mv boli",
        "microsoft sans serif",
    ];
    if script_or_symbol.iter().any(|s| f.contains(s)) {
        return 0.72;
    }

    let document_faces = [
        "arial", "times new roman", "calibri", "cambria", "candara", "constantia", "corbel",
        "courier", "georgia", "garamond", "palatino", "book antiqua", "bookman", "segoe ui",
        "tahoma", "trebuchet", "verdana", "helvetica", "franklin gothic", "century", "rockwell",
        "comic sans", "consolas", "lucida console", "lucida sans", "sitka", "bahnschrift",
        "cascadia", "impact", "roboto", "open sans", "lato", "noto s", "liberation", "tinos",
        "arimo", "carlito", "dejavu",
    ];
    if document_faces.iter().any(|s| f.contains(s)) {
        return 1.0;
    }

    0.88
}

fn family_of(font: &Font, path: &Path) -> String {
    font.name()
        .map(|n| n.to_string())
        .unwrap_or_else(|| path.file_stem().unwrap_or_default().to_string_lossy().to_string())
}

/// Ranks `candidates` against a single scanned word. Used at edit time, so a
/// correction is set in the face of *that* word: a bold heading and the body
/// text of the same page are not the same font.
pub fn rank_for_word(image: &GrayImage, word: &OcrWord, candidates: &[PathBuf]) -> Vec<FontMatch> {
    let Some(scanned) = word_sample(image, word) else { return Vec::new() };

    let mut matches: Vec<FontMatch> = candidates
        .iter()
        .filter_map(|path| {
            let font = load_font(path)?;
            let rendered = render_sample(&font, &word.text, scanned.src_height)?;
            let family = family_of(&font, path);
            let score = similarity(&scanned, &rendered) * usage_prior(&family);
            Some(FontMatch { path: path.clone(), family, score })
        })
        .filter(|m| m.score > 0.0)
        .collect();

    matches.sort_by(|a, b| b.score.total_cmp(&a.score));
    matches
}

fn samples<'a>(image: &GrayImage, words: &'a [OcrWord]) -> Vec<(&'a OcrWord, Sample)> {
    let mut usable: Vec<&OcrWord> = words
        .iter()
        .filter(|w| {
            w.text.chars().count() >= MIN_WORD_LEN
                && w.text.chars().all(|c| c.is_alphanumeric())
                && w.bbox.2 >= 12.0
                && w.bbox.3 >= 12.0
        })
        .collect();
    usable.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));

    usable
        .into_iter()
        .filter_map(|w| word_sample(image, w).map(|s| (w, s)))
        .take(MAX_WORDS)
        .collect()
}

/// Ranks `candidates` against the page, best faces first.
///
/// Two passes: every candidate is scored cheaply on one word, and only the best
/// few are then scored properly on all of them. With a few hundred fonts
/// installed, doing the full comparison everywhere would be far too slow.
pub fn rank(image: &GrayImage, words: &[OcrWord], candidates: &[PathBuf]) -> Vec<FontMatch> {
    let samples = samples(image, words);
    let Some((probe_word, probe)) = samples.first() else { return Vec::new() };
    let probe_bin = probe.binarized();
    let probe_ink = probe.ink_fraction();

    let mut fonts: Vec<(PathBuf, Font)> = candidates
        .iter()
        .filter_map(|p| load_font(p).map(|f| (p.clone(), f)))
        .collect();

    let mut shortlist: Vec<(f32, usize)> = fonts
        .iter()
        .enumerate()
        .filter_map(|(i, (_, font))| {
            let rendered = render_sample(font, &probe_word.text, probe.src_height)?;
            Some((quick_score(&probe_bin, probe_ink, &rendered), i))
        })
        .filter(|(s, _)| *s > 0.0)
        .collect();
    shortlist.sort_by(|a, b| b.0.total_cmp(&a.0));
    shortlist.truncate(SHORTLIST);

    let mut matches: Vec<FontMatch> = shortlist
        .iter()
        .filter_map(|(_, i)| {
            let (path, font) = &fonts[*i];
            let mut cache: HashMap<&str, Sample> = HashMap::new();

            let mut scores: Vec<f32> = Vec::with_capacity(samples.len());
            for (word, scanned) in &samples {
                let rendered = match cache.get(word.text.as_str()) {
                    Some(s) => s.clone(),
                    None => {
                        let s = render_sample(font, &word.text, scanned.src_height)?;
                        cache.insert(word.text.as_str(), s.clone());
                        s
                    }
                };
                scores.push(similarity(scanned, &rendered));
            }
            if scores.is_empty() {
                return None;
            }
            // Trimmed mean: a face is judged on the words it explains well.
            // Scanner damage is uneven — a plain mean lets the two most mangled
            // words on the page drown out the six clean ones.
            scores.sort_by(|a, b| b.total_cmp(a));
            let keep = (scores.len() * 2).div_ceil(3).max(1);
            let total: f32 = scores[..keep].iter().sum();
            let family = family_of(font, path);
            let score = total / keep as f32 * usage_prior(&family);
            Some(FontMatch { path: path.clone(), family, score })
        })
        .filter(|m| m.score > 0.0)
        .collect();

    fonts.clear();
    matches.sort_by(|a, b| b.score.total_cmp(&a.score));
    matches.truncate(8);
    matches
}

/// Best face for the page, or nothing if no candidate is convincing.
pub fn detect(image: &GrayImage, words: &[OcrWord], candidates: &[PathBuf]) -> Option<FontMatch> {
    rank(image, words, candidates).into_iter().next()
}

/// Every installed face belonging to `family` — its regular, bold, italic and
/// bold-italic cuts.
///
/// The family is settled once, on the whole page, where there is enough evidence
/// to be sure. A single word — and above all a short or numeric one, like a date
/// — carries far too little signal to pick a typeface on its own: let it choose
/// freely and each word of a line lands in a different font, which looks broken
/// however good each individual score is. Per word we therefore only decide the
/// *cut* within the family that the page already settled on.
pub fn family_variants(family: &str, candidates: &[PathBuf]) -> Vec<PathBuf> {
    candidates
        .iter()
        .filter(|path| {
            crate::pdf::ttf::TtfFont::load(path)
                .map(|f| f.family.eq_ignore_ascii_case(family))
                .unwrap_or(false)
        })
        .cloned()
        .collect()
}

/// The family the page is set in, from the ranking of individual faces.
pub fn page_family(ranking: &[FontMatch]) -> Option<String> {
    let best = ranking.first()?;
    crate::pdf::ttf::TtfFont::load(&best.path)
        .ok()
        .map(|f| f.family)
        .or_else(|| Some(best.family.clone()))
}
