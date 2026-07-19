// =============================================================================
// PDF Lks Text Editor - Exemple : Benchmark de reconnaissance de polices
// Créé par Leksa667 (https://github.com/Leksa667)
// =============================================================================

use fontdue::Font;
use image::GrayImage;
use pdf_editor::ocr::fontmatch;
use pdf_editor::ocr::tesseract::OcrWord;
use pdf_editor::pdf::ttf::TtfFont;
use std::collections::HashMap;
use std::path::PathBuf;

const WORDS: &[&str] = &["planning", "vendredi", "horaires", "semaine", "juillet", "matin"];
const CAP_PX: f32 = 22.0;

fn simulate_scan(font: &Font, words: &[&str], seed: u32) -> (GrayImage, Vec<OcrWord>) {
    let size = CAP_PX * 1.45;
    let margin = 20i32;
    let line_h = (size * 1.6) as i32;
    let mut width = 0usize;
    for w in words {
        let mut pen = 0.0;
        for c in w.chars() {
            pen += font.metrics(c, size).advance_width;
        }
        width = width.max(pen.ceil() as usize);
    }
    let img_w = (width + margin as usize * 2 + 8) as u32;
    let img_h = (words.len() as i32 * line_h + margin * 2) as u32;

    let mut canvas = vec![0.0f32; (img_w * img_h) as usize];
    let mut boxes = Vec::new();

    for (li, w) in words.iter().enumerate() {
        let baseline = margin + (li as i32 + 1) * line_h - (size * 0.4) as i32;
        let mut pen = margin as f32;
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for c in w.chars() {
            let (m, bitmap) = font.rasterize(c, size);
            let gx0 = pen + m.xmin as f32;
            let gy0 = baseline as f32 - m.ymin as f32 - m.height as f32;
            for y in 0..m.height {
                for x in 0..m.width {
                    let px = (gx0 as i32 + x as i32) as u32;
                    let py = (gy0 as i32 + y as i32) as u32;
                    if px < img_w && py < img_h {
                        let v = bitmap[y * m.width + x] as f32 / 255.0;
                        let slot = &mut canvas[(py * img_w + px) as usize];
                        *slot = slot.max(v);
                    }
                }
            }
            if m.width > 0 && m.height > 0 {
                x0 = x0.min(gx0);
                y0 = y0.min(gy0);
                x1 = x1.max(gx0 + m.width as f32);
                y1 = y1.max(gy0 + m.height as f32);
            }
            pen += m.advance_width;
        }
        if x1 > x0 {
            boxes.push(OcrWord {
                text: w.to_string(),
                bbox: (x0 - 1.0, y0 - 1.0, x1 - x0 + 2.0, y1 - y0 + 2.0),
                confidence: 90.0,
            });
        }
    }

    // Scanner degradation: gaussian-ish blur, contrast squeeze, noise.
    let mut blurred = vec![0.0f32; canvas.len()];
    for y in 0..img_h as i32 {
        for x in 0..img_w as i32 {
            let mut sum = 0.0;
            let mut wsum = 0.0;
            for dy in -1..=1i32 {
                for dx in -1..=1i32 {
                    let wgt = if dx == 0 && dy == 0 { 2.0 } else { 1.0 };
                    let (sx, sy) = (x + dx, y + dy);
                    if sx >= 0 && sy >= 0 && sx < img_w as i32 && sy < img_h as i32 {
                        sum += wgt * canvas[(sy as u32 * img_w + sx as u32) as usize];
                        wsum += wgt;
                    }
                }
            }
            blurred[(y as u32 * img_w + x as u32) as usize] = sum / wsum;
        }
    }

    let mut rng = seed.wrapping_mul(747796405).wrapping_add(2891336453);
    let mut img = GrayImage::new(img_w, img_h);
    for (i, v) in blurred.iter().enumerate() {
        rng = rng.wrapping_mul(747796405).wrapping_add(2891336453);
        let noise = ((rng >> 16) & 31) as f32 - 15.5;
        let gray = 235.0 - v * 205.0 + noise;
        img.as_mut()[i] = gray.clamp(0.0, 255.0) as u8;
    }
    (img, boxes)
}

fn main() {
    let candidates = pdf_editor::fontstore::candidates();

    // One regular cut per family.
    let mut families: HashMap<String, PathBuf> = HashMap::new();
    for path in &candidates {
        if let Ok(f) = TtfFont::load(path) {
            if !f.bold && !f.italic {
                families.entry(f.family.clone()).or_insert_with(|| path.clone());
            }
        }
    }
    let mut tests: Vec<(String, PathBuf)> = families.into_iter().collect();
    tests.sort();

    let mut total = 0usize;
    let mut correct = 0usize;
    let mut top3 = 0usize;
    let mut equivalent = 0usize;
    let mut doc_total = 0usize;
    let mut doc_ok = 0usize;
    let mut misses: Vec<(String, String, f32, bool)> = Vec::new();

    for (i, (family, path)) in tests.iter().enumerate() {
        let Ok(data) = std::fs::read(path) else { continue };
        let Ok(font) = Font::from_bytes(data, fontdue::FontSettings::default()) else { continue };
        // Skip symbol/decorative fonts that can't render our latin words.
        if WORDS[0].chars().any(|c| font.lookup_glyph_index(c) == 0) {
            continue;
        }

        let (img, words) = simulate_scan(&font, WORDS, i as u32 + 1);
        let ranking = fontmatch::rank(&img, &words, &candidates);
        let Some(best) = ranking.first() else {
            total += 1;
            misses.push((family.clone(), "<aucun>".into(), 0.0, false));
            continue;
        };

        total += 1;
        let is_doc_face = fontmatch::usage_prior(family) >= 1.0;
        if is_doc_face {
            doc_total += 1;
        }
        let got_family = TtfFont::load(&best.path).map(|f| f.family).unwrap_or_default();
        if got_family.eq_ignore_ascii_case(family) {
            correct += 1;
            top3 += 1;
            equivalent += 1;
            if is_doc_face {
                doc_ok += 1;
            }
        } else {
            let in_top3 = ranking.iter().take(3).any(|m| {
                TtfFont::load(&m.path)
                    .map(|f| f.family.eq_ignore_ascii_case(family))
                    .unwrap_or(false)
            });
            if in_top3 {
                top3 += 1;
            }
            // Visual equivalence: if the predicted face draws the same
            // letterforms as the truth (metric clones like Tinos / Times New
            // Roman or Ebrima / Segoe UI), the replacement is indistinguishable.
            let equiv = (|| {
                let pred = std::fs::read(&best.path).ok()?;
                let pred = Font::from_bytes(pred, fontdue::FontSettings::default()).ok()?;
                let data = std::fs::read(path).ok()?;
                let truth = Font::from_bytes(data, fontdue::FontSettings::default()).ok()?;
                let a = fontmatch::render_sample_debug(&truth, "planning", 30)?;
                let b = fontmatch::render_sample_debug(&pred, "planning", 30)?;
                Some(fontmatch::similarity(&a, &b) >= 0.85)
            })()
            .unwrap_or(false);
            if equiv {
                equivalent += 1;
                if is_doc_face {
                    doc_ok += 1;
                }
            }
            misses.push((family.clone(), got_family, best.score, equiv));
        }
    }

    println!("\n=== Résultats ===");
    println!("familles testées : {total}");
    println!(
        "top-1 : {correct} ({:.1}%)   top-3 : {top3} ({:.1}%)   équivalence visuelle : {equivalent} ({:.1}%)",
        correct as f32 * 100.0 / total.max(1) as f32,
        top3 as f32 * 100.0 / total.max(1) as f32,
        equivalent as f32 * 100.0 / total.max(1) as f32
    );
    println!(
        "polices de document (prior 1.0) : {doc_ok}/{doc_total} ({:.1}%)",
        doc_ok as f32 * 100.0 / doc_total.max(1) as f32
    );
    if !misses.is_empty() {
        println!("\nErreurs :");
        for (want, got, score, equiv) in &misses {
            let tag = if *equiv { "  [clone visuel]" } else { "" };
            println!("  {want}  ->  {got}  ({:.2}){tag}", score);
        }
    }
}
