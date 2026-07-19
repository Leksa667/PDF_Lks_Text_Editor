// =============================================================================
// PDF Lks Text Editor - Exemple : Diagnostic de similarité de polices
// Créé par Leksa667 (https://github.com/Leksa667)
// =============================================================================

use fontdue::Font;
use image::GrayImage;
use pdf_editor::ocr::fontmatch::{self, Sample};
use pdf_editor::ocr::tesseract::OcrWord;

const CAP_PX: f32 = 22.0;

fn simulate_scan(font: &Font, word: &str) -> (GrayImage, OcrWord) {
    let size = CAP_PX * 1.45;
    let margin = 20i32;
    let mut pen = margin as f32;
    let baseline = 80i32;
    let mut img_w = 0f32;
    for c in word.chars() {
        img_w += font.metrics(c, size).advance_width;
    }
    let img_w = (img_w + margin as f32 * 2.0) as u32;
    let img_h = 140u32;
    let mut canvas = vec![0.0f32; (img_w * img_h) as usize];
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for c in word.chars() {
        let (m, bitmap) = font.rasterize(c, size);
        let gx0 = pen + m.xmin as f32;
        let gy0 = baseline as f32 - m.ymin as f32 - m.height as f32;
        for y in 0..m.height {
            for x in 0..m.width {
                let px = (gx0 as i32 + x as i32) as u32;
                let py = (gy0 as i32 + y as i32) as u32;
                if px < img_w && py < img_h {
                    let v = bitmap[y * m.width + x] as f32 / 255.0;
                    let s = &mut canvas[(py * img_w + px) as usize];
                    *s = s.max(v);
                }
            }
        }
        if m.width > 0 {
            x0 = x0.min(gx0);
            y0 = y0.min(gy0);
            x1 = x1.max(gx0 + m.width as f32);
            y1 = y1.max(gy0 + m.height as f32);
        }
        pen += m.advance_width;
    }
    let mut blurred = vec![0.0f32; canvas.len()];
    for y in 0..img_h as i32 {
        for x in 0..img_w as i32 {
            let mut sum = 0.0;
            let mut wsum = 0.0;
            for dy in -1..=1i32 {
                for dx in -1..=1i32 {
                    let w = if dx == 0 && dy == 0 { 2.0 } else { 1.0 };
                    let (sx, sy) = (x + dx, y + dy);
                    if sx >= 0 && sy >= 0 && sx < img_w as i32 && sy < img_h as i32 {
                        sum += w * canvas[(sy as u32 * img_w + sx as u32) as usize];
                        wsum += w;
                    }
                }
            }
            blurred[(y as u32 * img_w + x as u32) as usize] = sum / wsum;
        }
    }
    let mut rng = 12345u32;
    let mut img = GrayImage::new(img_w, img_h);
    for (i, v) in blurred.iter().enumerate() {
        rng = rng.wrapping_mul(747796405).wrapping_add(2891336453);
        let noise = ((rng >> 16) & 31) as f32 - 15.5;
        img.as_mut()[i] = (235.0 - v * 205.0 + noise).clamp(0.0, 255.0) as u8;
    }
    let w = OcrWord {
        text: word.to_string(),
        bbox: (x0 - 1.0, y0 - 1.0, x1 - x0 + 2.0, y1 - y0 + 2.0),
        confidence: 90.0,
    };
    (img, w)
}

fn dump(tag: &str, s: &Sample) {
    println!("{tag}: w={} src_h={} ink={:.3}", s.width(), s.src_height(), s.ink());
    for y in (0..40).step_by(2) {
        let mut line = String::new();
        for x in (0..s.width()).step_by(2) {
            let v = s.value(x, y);
            line.push(if v > 0.6 {
                '#'
            } else if v > 0.25 {
                '+'
            } else {
                ' '
            });
        }
        println!("  |{line}|");
    }
}

fn main() {
    let times = std::fs::read(r"C:\Windows\Fonts\times.ttf").unwrap();
    let times = Font::from_bytes(times, fontdue::FontSettings::default()).unwrap();
    let (img, word) = simulate_scan(&times, "planning");
    let scanned = fontmatch::word_sample(&img, &word).unwrap();
    for rival in [
        r"C:\Windows\Fonts\times.ttf",
        r"C:\Windows\Fonts\himalaya.ttf",
        r"C:\Windows\Fonts\calibri.ttf",
    ] {
        let data = std::fs::read(rival).unwrap();
        let font = Font::from_bytes(data, fontdue::FontSettings::default()).unwrap();
        let rendered =
            fontmatch::render_sample_debug(&font, "planning", scanned.src_height()).unwrap();
        let (shape, profile, wr, dens) = fontmatch::similarity_parts(&scanned, &rendered);
        println!(
            "TNR-scan vs {rival}: sim={:.3} shape={:.3} profile={:.3} wr={:.3} dens={:.3}",
            fontmatch::similarity(&scanned, &rendered),
            shape,
            profile,
            wr,
            dens
        );
        dump("rendu", &rendered);
    }

    for (name, file) in [
        ("Calibri", r"C:\Windows\Fonts\calibri.ttf"),
        ("Verdana", r"C:\Windows\Fonts\verdana.ttf"),
        ("Arial", r"C:\Windows\Fonts\arial.ttf"),
        ("Times New Roman", r"C:\Windows\Fonts\times.ttf"),
        ("Segoe UI", r"C:\Windows\Fonts\segoeui.ttf"),
    ] {
        let data = std::fs::read(file).unwrap();
        let font = Font::from_bytes(data, fontdue::FontSettings::default()).unwrap();

        let (img, word) = simulate_scan(&font, "planning");
        let scanned = fontmatch::word_sample(&img, &word).unwrap();
        let rendered =
            fontmatch::render_sample_debug(&font, "planning", scanned.src_height()).unwrap();
        let (shape, profile, wr, density) = fontmatch::similarity_parts(&scanned, &rendered);
        println!(
            "{name}: self-sim={:.3}  scan w={} render w={}  shape={:.3} profile={:.3} wr={:.3} dens={:.3}",
            fontmatch::similarity(&scanned, &rendered),
            scanned.width(),
            rendered.width(),
            shape,
            profile,
            wr,
            density
        );

        let candidates = pdf_editor::fontstore::candidates();
        let ranking = fontmatch::rank(&img, &[word], &candidates);
        for (i, m) in ranking.iter().take(5).enumerate() {
            println!("   #{}: {}  {:.3}", i + 1, m.family, m.score);
        }
        if name == "Verdana" {
            let scan_bin = scanned.clone();
            dump("scan(verdana)", &scan_bin);
            dump("rendu(verdana)", &rendered);
        }
        println!();
    }
}
