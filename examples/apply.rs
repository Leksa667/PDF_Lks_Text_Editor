// =============================================================================
// PDF Lks Text Editor - Exemple : Application d'éditions par lots sur un PDF scanné
// Créé par Leksa667 (https://github.com/Leksa667)
// =============================================================================

use pdf_editor::ocr::tesseract::{ocr_word_positions, OcrWord};
use pdf_editor::pdf::document::PdfDocument;
use pdf_editor::pdf::overlay::{paint_line, visual_size, PaintFont, WordBox};
use pdf_editor::pdf::render::render_page_to_png_bytes;
use pdf_editor::pdf::ttf::TtfFont;

const OCR_DPI: u32 = 300;

fn main() {
    let src = std::path::PathBuf::from(std::env::args().nth(1).unwrap());
    let out = std::path::PathBuf::from(std::env::args().nth(2).unwrap());

    let mut doc = PdfDocument::open(&src).unwrap();

    // The scan's typeface is a narrow sans: use Arial (matches, never overflows).
    let font = TtfFont::load(std::path::Path::new(r"C:\Windows\Fonts\arial.ttf")).unwrap();

    // Edits to apply, in order.
    let edits: &[(&str, &str)] = &[
        ("04/04/25", "07/07/26"),
        ("31", "32"),
        ("lundi", "vendredi"),
        ("avril", "juillet"),
        ("2025", "2026"),
    ];

    for page in 0..doc.page_count() {
        if !doc.is_scanned(page) {
            continue;
        }
        let geometry = doc.geometry(page).unwrap();
        let page_id = doc.page_id(page).unwrap();

        // OCR the page.
        let bytes = render_page_to_png_bytes(&src, page as u32 + 1, OCR_DPI).unwrap();
        let png = std::env::temp_dir().join(format!("apply_{page}.png"));
        std::fs::write(&png, &bytes).unwrap();
        let mut words = match ocr_word_positions(&png, "fra+eng") {
            Ok(w) => w,
            Err(_) => continue,
        };

        let scale = 72.0 / OCR_DPI as f32;
        let (_, vh) = visual_size(&geometry);
        let to_visual = |w: &OcrWord| {
            [
                w.bbox.0 * scale,
                vh - (w.bbox.1 + w.bbox.3) * scale,
                (w.bbox.0 + w.bbox.2) * scale,
                vh - w.bbox.1 * scale,
            ]
        };

        for (from, to) in edits {
            loop {
                let Some(index) = words.iter().position(|w| &w.text == from) else { break };
                let rect = to_visual(&words[index]);
                let line_h = rect[3] - rect[1];
                let mid_x = (rect[0] + rect[2]) / 2.0;

                // Same-line followers to the right.
                let mut followers: Vec<(f32, usize)> = words
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != index)
                    .filter_map(|(i, w)| {
                        let r = to_visual(w);
                        let overlap = (rect[3].min(r[3]) - rect[1].max(r[1])).max(0.0);
                        let same = overlap > line_h.min(r[3] - r[1]) * 0.4;
                        (same && r[0] > mid_x).then_some((r[0], i))
                    })
                    .collect();
                followers.sort_by(|a, b| a.0.total_cmp(&b.0));

                let (_, needed, _, _) = pdf_editor::pdf::overlay::fit_to_box(&font, to, line_h);
                let next_left = followers.first().map(|(x, _)| *x);
                let gap = line_h * 0.35;
                let available = next_left.map(|nl| nl - gap - rect[0]).unwrap_or(f32::MAX);

                if next_left.is_none() || needed <= available.max(rect[2] - rect[0]) {
                    let grown = (rect[0] + needed).min(next_left.map(|nl| nl - gap * 0.4).unwrap_or(f32::MAX));
                    let mask_right = grown.max(rect[2]);
                    let wb = WordBox { rect: [rect[0], rect[1], mask_right, rect[3]], text: (*to).into() };
                    paint_line(&mut doc.doc, page_id, &geometry, std::slice::from_ref(&wb),
                               Some(mask_right), [1.0, 1.0, 1.0], PaintFont::Embedded(&font)).unwrap();
                    words[index].text = (*to).into();
                } else {
                    let mut text = to.to_string();
                    let mut last_right = rect[2];
                    let (mut lb, mut lt) = (rect[1], rect[3]);
                    let mut absorbed: Vec<usize> = Vec::new();
                    for (_, i) in &followers {
                        let r = to_visual(&words[*i]);
                        last_right = r[2];
                        lb = lb.min(r[1]); lt = lt.max(r[3]);
                        text.push(' '); text.push_str(&words[*i].text);
                        absorbed.push(*i);
                    }
                    let wb = WordBox { rect: [rect[0], lb, last_right, lt], text };
                    paint_line(&mut doc.doc, page_id, &geometry, std::slice::from_ref(&wb),
                               Some(last_right), [1.0, 1.0, 1.0], PaintFont::Embedded(&font)).unwrap();
                    words[index].text = (*to).into();
                    absorbed.sort_unstable_by(|a, b| b.cmp(a));
                    for i in absorbed { words.remove(i); }
                }
                println!("page {} : « {from} » -> « {to} »", page + 1);
            }
        }
    }

    doc.save(&out).unwrap();
    println!("\nEnregistré : {}", out.display());
}
