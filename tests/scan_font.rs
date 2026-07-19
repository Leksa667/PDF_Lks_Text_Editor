// =============================================================================
// PDF Lks Text Editor - Tests : Reconnaissance de polices sur scans
// Créé par Leksa667 (https://github.com/Leksa667)
// =============================================================================

use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document, Object, ObjectId, Stream};
use pdf_editor::ocr::fontmatch::detect;
use pdf_editor::ocr::tesseract::OcrWord;
use pdf_editor::pdf::overlay::{paint_text_region, PaintFont};
use pdf_editor::pdf::text::{extract_runs, pdf_rect_to_image};
use pdf_editor::pdf::ttf::TtfFont;
use std::path::{Path, PathBuf};

const DPI: u32 = 300;

/// Geometry of the unrotated 612x792 test page.
fn geom0() -> pdf_editor::pdf::document::PageGeometry {
    pdf_editor::pdf::document::PageGeometry { x0: 0.0, y0: 0.0, width: 612.0, height: 792.0, rotate: 0 }
}

fn system_font(name: &str) -> Option<PathBuf> {
    let path = Path::new(r"C:\Windows\Fonts").join(name);
    path.is_file().then_some(path)
}

fn blank_page() -> (Document, ObjectId) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let content = Content { operations: vec![] };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
        "Resources" => dictionary! {},
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    (doc, page_id)
}

/// Renders a page set in `ttf`, then asks the matcher which font it was,
/// pretending the rendered words came out of the OCR.
fn identify(ttf_path: &Path, words: &[&str], candidates: &[PathBuf]) -> (String, PathBuf, f32) {
    let font = TtfFont::load(ttf_path).unwrap();
    let (mut doc, page_id) = blank_page();

    let mut regions = Vec::new();
    for (i, word) in words.iter().enumerate() {
        let y = 700.0 - i as f32 * 60.0;
        // A real scan is not stretched, so the box must be exactly the ink box of
        // the word in that font — which is also what an OCR box would report.
        let height = 30.0;
        let (_, ink_width, _, _) = pdf_editor::pdf::overlay::fit_to_box(&font, word, height);
        let rect = [60.0, y, 60.0 + ink_width, y + height];
        paint_text_region(&mut doc, page_id, &geom0(), rect, word, [1.0, 1.0, 1.0], PaintFont::Embedded(&font))
            .unwrap();
        regions.push(rect);
    }

    let path = std::env::temp_dir().join(format!(
        "pdf_editor_scan_{}.pdf",
        ttf_path.file_stem().unwrap().to_string_lossy()
    ));
    doc.save(&path).unwrap();

    let bytes = pdf_editor::pdf::render::render_page_to_png_bytes(&path, 1, DPI).unwrap();
    let image = image::load_from_memory(&bytes).unwrap().to_luma8();

    let geometry = pdf_editor::pdf::document::PdfDocument::open(&path)
        .unwrap()
        .geometry(0)
        .unwrap();

    let ocr_words: Vec<OcrWord> = words
        .iter()
        .zip(&regions)
        .map(|(text, rect)| {
            let r = pdf_rect_to_image(&geometry, DPI, *rect);
            OcrWord {
                text: (*text).to_string(),
                bbox: (r[0], r[1], r[2] - r[0], r[3] - r[1]),
                confidence: 95.0,
            }
        })
        .collect();

    let found = detect(&image, &ocr_words, candidates).expect("aucune police reconnue");
    (found.family, found.path, found.score)
}

#[test]
fn recognises_which_font_a_scan_was_set_in() {
    if !pdf_editor::pdf::render::is_renderer_available() {
        eprintln!("moteur de rendu absent, test ignoré");
        return;
    }
    let (Some(arial), Some(times), Some(courier)) = (
        system_font("arial.ttf"),
        system_font("times.ttf"),
        system_font("cour.ttf"),
    ) else {
        eprintln!("polices système absentes, test ignoré");
        return;
    };
    let candidates = vec![arial.clone(), times.clone(), courier.clone()];
    let words = ["Facture", "Montant", "Livraison"];

    let (family, _, score) = identify(&arial, &words, &candidates);
    assert!(family.contains("Arial"), "police Arial mal reconnue : {family} ({score:.2})");
    assert!(score > 0.7, "score trop bas : {score:.2}");

    let (family, _, score) = identify(&times, &words, &candidates);
    assert!(family.contains("Times"), "police Times mal reconnue : {family} ({score:.2})");

    let (family, _, _) = identify(&courier, &words, &candidates);
    assert!(family.contains("Courier"), "police Courier mal reconnue : {family}");
}

#[test]
fn embedded_scan_text_is_extractable_and_editable() {
    let Some(arial) = system_font("arial.ttf") else {
        eprintln!("arial.ttf absent, test ignoré");
        return;
    };
    let font = TtfFont::load(&arial).unwrap();
    assert!(font.family.contains("Arial"));
    assert!(font.gid('é').is_some());

    let (mut doc, page_id) = blank_page();
    paint_text_region(
        &mut doc,
        page_id,
        &geom0(),
        [60.0, 700.0, 300.0, 730.0],
        "Créé le 14 juillet",
        [1.0, 1.0, 1.0],
        PaintFont::Embedded(&font),
    )
    .unwrap();

    // The font travels with the file, and the painted text is a real run.
    let runs = extract_runs(&doc, page_id).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].text, "Créé le 14 juillet");
    assert!(runs[0].two_byte, "doit être un Type0/Identity-H");
    assert_eq!(runs[0].map_source, "ToUnicode");
    assert!(runs[0].editable);
    assert!(runs[0].base_font.contains("Arial"));

    // Re-editing it goes through the normal text engine, in the embedded font.
    let run = runs[0].clone();
    pdf_editor::pdf::edit::set_run_text(
        &mut doc,
        page_id,
        run.op_index,
        &run.font_res,
        run.two_byte,
        "Créé le 15 juillet",
    )
    .unwrap();
    assert_eq!(extract_runs(&doc, page_id).unwrap()[0].text, "Créé le 15 juillet");

    // And the font program really travels inside the PDF.
    let embedded = doc.objects.values().any(|o| {
        o.as_stream()
            .map(|s| s.dict.has(b"Length1") && s.content.len() > 5_000)
            .unwrap_or(false)
    });
    assert!(embedded, "le programme de police n'est pas embarqué");
}

#[test]
fn downloads_an_open_licence_font_into_the_cache() {
    let entry = &pdf_editor::fontstore::CATALOG[1]; // Tinos: métriques de Times
    let path = match pdf_editor::fontstore::download(entry) {
        Ok(path) => path,
        Err(e) => {
            eprintln!("téléchargement indisponible ({e:#}), test ignoré");
            return;
        }
    };
    assert!(path.is_file());

    let font = TtfFont::load(&path).unwrap();
    assert!(font.family.contains("Tinos"), "famille inattendue : {}", font.family);
    assert!(font.gid('é').is_some());
    assert!(pdf_editor::fontstore::is_cached(entry));

    // A downloaded font must be usable straight away, without installing it.
    assert!(pdf_editor::fontstore::candidates().contains(&path));
}

/// Ink bounding box of the rendered page, in image pixels.
fn ink_bbox(doc: &mut Document, name: &str) -> [f32; 4] {
    let path = std::env::temp_dir().join(name);
    doc.save(&path).unwrap();
    let bytes = pdf_editor::pdf::render::render_page_to_png_bytes(&path, 1, DPI).unwrap();
    let img = image::load_from_memory(&bytes).unwrap().to_luma8();

    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (x, y, p) in img.enumerate_pixels() {
        if p[0] < 128 {
            x0 = x0.min(x as f32);
            y0 = y0.min(y as f32);
            x1 = x1.max(x as f32 + 1.0);
            y1 = y1.max(y as f32 + 1.0);
        }
    }
    [x0, y0, x1, y1]
}

#[test]
fn repainted_text_lands_exactly_in_the_ocr_box() {
    if !pdf_editor::pdf::render::is_renderer_available() {
        eprintln!("moteur de rendu absent, test ignoré");
        return;
    }
    let Some(arial) = system_font("arial.ttf") else {
        eprintln!("arial.ttf absent, test ignoré");
        return;
    };
    let font = TtfFont::load(&arial).unwrap();

    // "case" has neither ascender nor descender: its OCR box is only as tall as
    // the x-height, which is exactly the case that naive sizing gets wrong.
    for (i, word) in ["case", "Réglage"].iter().enumerate() {
        let region = [80.0, 600.0, 260.0, 620.0];
        let (mut doc, page_id) = blank_page();
        paint_text_region(&mut doc, page_id, &geom0(), region, word, [1.0, 1.0, 1.0], PaintFont::Embedded(&font))
            .unwrap();

        let geometry = {
            let path = std::env::temp_dir().join("pdf_editor_box_probe.pdf");
            doc.save(&path).unwrap();
            pdf_editor::pdf::document::PdfDocument::open(&path).unwrap().geometry(0).unwrap()
        };
        let expected = pdf_rect_to_image(&geometry, DPI, region);
        let actual = ink_bbox(&mut doc, &format!("pdf_editor_box_{i}.pdf"));

        // The correction is placed in the box, left-aligned and vertically
        // filling it, but is deliberately NOT stretched to reach the right edge:
        // distorting glyphs to fill a fixed box is exactly what made corrections
        // look ugly. So the ink must stay inside the box (no overflow) and start
        // at its left, without being forced to its right edge.
        let tol = 5.0;
        assert!((actual[0] - expected[0]).abs() <= tol, "« {word} » : bord gauche {} vs {}", actual[0], expected[0]);
        assert!((actual[1] - expected[1]).abs() <= tol, "« {word} » : bord haut");
        assert!((actual[3] - expected[3]).abs() <= tol, "« {word} » : bord bas");
        assert!(actual[2] <= expected[2] + tol, "« {word} » : le texte déborde à droite ({} > {})", actual[2], expected[2]);
    }
}

#[test]
fn embedded_font_is_subset_not_the_whole_file() {
    let Some(arial) = system_font("arial.ttf") else {
        eprintln!("arial.ttf absent, test ignoré");
        return;
    };
    let original = std::fs::metadata(&arial).unwrap().len();
    let font = TtfFont::load(&arial).unwrap();

    let mut chars: std::collections::BTreeSet<char> = "Créé le 14 juillet".chars().collect();
    chars.extend(pdf_editor::pdf::subset::base_charset());
    let subsetted = pdf_editor::pdf::subset::subset(&font, &chars).unwrap();

    assert!(
        (subsetted.program.len() as u64) < original / 10,
        "subset trop gros : {} vs {} octets",
        subsetted.program.len(),
        original
    );

    // Glyphs are renumbered compactly, and every kept character has an id.
    assert!(subsetted.gid_map.len() <= chars.len() + 8, "trop de glyphes conservés");
    for c in ["é", "A", "W", "€"].iter().flat_map(|s| s.chars()) {
        let old = font.gid(c).unwrap();
        assert!(subsetted.gid_map.contains_key(&old), "glyphe manquant : {c}");
    }
    // Glyph 0 (.notdef) must stay at id 0.
    assert_eq!(subsetted.gid_map.get(&0), Some(&0));
}

#[test]
fn subsetted_pdf_stays_small_and_still_renders_the_text() {
    if !pdf_editor::pdf::render::is_renderer_available() {
        eprintln!("moteur de rendu absent, test ignoré");
        return;
    }
    let Some(arial) = system_font("arial.ttf") else {
        eprintln!("arial.ttf absent, test ignoré");
        return;
    };
    let original = std::fs::metadata(&arial).unwrap().len();
    let font = TtfFont::load(&arial).unwrap();

    let region = [80.0, 600.0, 320.0, 622.0];
    let (mut doc, page_id) = blank_page();
    paint_text_region(&mut doc, page_id, &geom0(), region, "Créé le 14 juillet", [1.0, 1.0, 1.0], PaintFont::Embedded(&font))
        .unwrap();

    // The subsetted font really renders: the ink lands in the box.
    let ink = ink_bbox(&mut doc, "pdf_editor_subset_render.pdf");
    let probe = std::env::temp_dir().join("pdf_editor_subset_render.pdf");
    let geometry = pdf_editor::pdf::document::PdfDocument::open(&probe).unwrap().geometry(0).unwrap();
    let expected = pdf_rect_to_image(&geometry, DPI, region);
    // Left-aligned, vertically filling the box, and never overflowing it.
    assert!((ink[0] - expected[0]).abs() <= 6.0, "gauche : {ink:?} vs {expected:?}");
    assert!((ink[1] - expected[1]).abs() <= 6.0, "haut : {ink:?} vs {expected:?}");
    assert!((ink[3] - expected[3]).abs() <= 6.0, "bas : {ink:?} vs {expected:?}");
    assert!(ink[2] <= expected[2] + 6.0, "déborde à droite : {ink:?} vs {expected:?}");

    // Editing again reuses the same subset: no second copy of the font.
    let run = extract_runs(&doc, page_id).unwrap().remove(0);
    pdf_editor::pdf::edit::set_run_text(
        &mut doc,
        page_id,
        run.op_index,
        &run.font_res,
        run.two_byte,
        "Créé le 15 août",
    )
    .unwrap();

    let path = std::env::temp_dir().join("pdf_editor_subset.pdf");
    doc.save(&path).unwrap();
    let size = std::fs::metadata(&path).unwrap().len();
    assert!(
        size < original / 4,
        "PDF trop gros : {size} octets pour une police de {original}"
    );

    assert_eq!(extract_runs(&doc, page_id).unwrap()[0].text, "Créé le 15 août");
}

/// A page whose /Rotate makes the reader see it turned — exactly the case of a
/// landscape scan produced by a copier.
fn rotated_page(rotate: i64) -> (Document, ObjectId, pdf_editor::pdf::document::PageGeometry) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let content = Content { operations: vec![] };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
        "Resources" => dictionary! {},
        "MediaBox" => vec![0.into(), 0.into(), 842.into(), 595.into()],
        "Rotate" => rotate,
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);

    let geometry = pdf_editor::pdf::document::PageGeometry {
        x0: 0.0,
        y0: 0.0,
        width: 842.0,
        height: 595.0,
        rotate,
    };
    (doc, page_id, geometry)
}

#[test]
fn text_painted_on_a_rotated_page_reads_upright_and_fills_its_box() {
    if !pdf_editor::pdf::render::is_renderer_available() {
        eprintln!("moteur de rendu absent, test ignoré");
        return;
    }
    let Some(arial) = system_font("arial.ttf") else {
        eprintln!("arial.ttf absent, test ignoré");
        return;
    };
    let font = TtfFont::load(&arial).unwrap();

    for rotate in [0, 90, 180, 270] {
        let (mut doc, page_id, geometry) = rotated_page(rotate);

        // Box expressed as the reader sees it: 200 pt wide, 24 pt tall.
        let region = [100.0, 300.0, 300.0, 324.0];
        paint_text_region(
            &mut doc,
            page_id,
            &geometry,
            region,
            "Facture 2026",
            [1.0, 1.0, 1.0],
            PaintFont::Embedded(&font),
        )
        .unwrap();

        let ink = ink_bbox(&mut doc, &format!("pdf_editor_rot_{rotate}.pdf"));

        // Ghostscript applies /Rotate when rendering, so the ink must land inside
        // the visual box — in visual pixels.
        let s = DPI as f32 / 72.0;
        let (_, visual_height) = pdf_editor::pdf::overlay::visual_size(&geometry);
        let expected = [
            region[0] * s,
            (visual_height - region[3]) * s,
            region[2] * s,
            (visual_height - region[1]) * s,
        ];
        for (axis, (a, e)) in ink.iter().zip(expected.iter()).enumerate() {
            assert!(
                (a - e).abs() <= 6.0,
                "/Rotate {rotate} : bord {axis} à {a:.0} px au lieu de {e:.0} px (encre {ink:?})",
            );
        }

        // Upright text is wider than it is tall: a sideways glyph run would fail this.
        let (w, h) = (ink[2] - ink[0], ink[3] - ink[1]);
        assert!(w > h * 3.0, "/Rotate {rotate} : le texte semble couché ({w:.0}x{h:.0})");
    }
}

/// Scanners commonly emit a page whose content stream *opens* with a `cm` that
/// is never closed by a `Q`. Since all content streams of a page are
/// concatenated, anything appended afterwards inherits that matrix: text painted
/// on such a scan came out at 36% of its size, far from its box, while the mask
/// missed the word entirely.
#[test]
fn a_scan_leaving_an_unbalanced_matrix_does_not_displace_the_correction() {
    if !pdf_editor::pdf::render::is_renderer_available() {
        eprintln!("moteur de rendu absent, test ignoré");
        return;
    }
    let Some(arial) = system_font("arial.ttf") else {
        eprintln!("arial.ttf absent, test ignoré");
        return;
    };
    let font = TtfFont::load(&arial).unwrap();

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    // The scanner's prologue: a scale that is never restored.
    let content = Content {
        operations: vec![
            Operation::new("cm", vec![0.36.into(), 0.into(), 0.into(), 0.36.into(), 0.into(), 0.into()]),
            Operation::new("q", vec![]),
            Operation::new("rg", vec![0.78.into(), 0.78.into(), 0.78.into()]),
            Operation::new("re", vec![0.into(), 0.into(), 1700.into(), 2200.into()]),
            Operation::new("f", vec![]),
            Operation::new("Q", vec![]),
        ],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
        "Resources" => dictionary! {},
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);

    let geometry = pdf_editor::pdf::document::PageGeometry {
        x0: 0.0,
        y0: 0.0,
        width: 612.0,
        height: 792.0,
        rotate: 0,
    };
    let region = [100.0, 400.0, 300.0, 424.0];
    paint_text_region(
        &mut doc,
        page_id,
        &geometry,
        region,
        "Correction",
        [1.0, 1.0, 1.0],
        PaintFont::Embedded(&font),
    )
    .unwrap();

    let ink = ink_bbox(&mut doc, "pdf_editor_unbalanced_cm.pdf");
    let s = DPI as f32 / 72.0;
    let expected = [
        region[0] * s,
        (792.0 - region[3]) * s,
        region[2] * s,
        (792.0 - region[1]) * s,
    ];
    // The point of this test is that the scanner's leftover matrix does not
    // displace or shrink the text: left/top/bottom must land on the box, and the
    // text must not spill past its right edge.
    assert!((ink[0] - expected[0]).abs() <= 6.0, "déplacé en x : {ink:?} vs {expected:?}");
    assert!((ink[1] - expected[1]).abs() <= 6.0, "déplacé en y : {ink:?} vs {expected:?}");
    assert!((ink[3] - expected[3]).abs() <= 6.0, "hauteur fausse : {ink:?} vs {expected:?}");
    assert!(ink[2] <= expected[2] + 6.0, "déborde : {ink:?} vs {expected:?}");
    // And it is genuinely full-size, not scaled down to 36% by the matrix.
    assert!(ink[2] - ink[0] > (expected[2] - expected[0]) * 0.5, "texte rétréci : {ink:?}");
}

/// The hard case: telling apart faces that actually look alike. Arial vs
/// Helvetica-like grotesques, a regular vs its own bold, a serif vs another
/// serif. Picking "some sans-serif" is not good enough.
#[test]
fn tells_close_typefaces_and_weights_apart() {
    if !pdf_editor::pdf::render::is_renderer_available() {
        eprintln!("moteur de rendu absent, test ignoré");
        return;
    }

    let wanted = [
        "arial.ttf", "arialbd.ttf", "calibri.ttf", "calibrib.ttf",
        "times.ttf", "timesbd.ttf", "verdana.ttf", "tahoma.ttf", "cour.ttf",
    ];
    let candidates: Vec<PathBuf> = wanted.iter().filter_map(|n| system_font(n)).collect();
    if candidates.len() < 6 {
        eprintln!("polices système insuffisantes, test ignoré");
        return;
    }

    let words = ["Toxicologie", "LABORATOIRE", "Resultats", "Analyse"];

    for target in &candidates {
        let (family, path, score) = identify(target, &words, &candidates);
        assert_eq!(
            &path,
            target,
            "{} confondue avec {family} ({score:.2})",
            target.file_name().unwrap().to_string_lossy()
        );
        assert!(score > 0.6, "{family} reconnue mais avec un score faible : {score:.2}");
    }
}

/// Rendering a scanned line: a short word replaced by a much longer one must not
/// collide with the next word. Either the correction grows into the blank space
/// after it, or the following words are shifted along — either way the ink stays
/// separated.
#[test]
fn a_longer_replacement_does_not_collide_with_the_next_word() {
    if !pdf_editor::pdf::render::is_renderer_available() {
        eprintln!("moteur de rendu absent, test ignoré");
        return;
    }
    let Some(arial) = system_font("arial.ttf") else {
        eprintln!("arial.ttf absent, test ignoré");
        return;
    };
    let font = TtfFont::load(&arial).unwrap();
    let geometry = pdf_editor::pdf::document::PageGeometry {
        x0: 0.0, y0: 0.0, width: 612.0, height: 792.0, rotate: 0,
    };

    use pdf_editor::pdf::overlay::{paint_line, WordBox};

    // "le" at [80,700]-[100,716], then a fixed neighbour "avril" at [150,700].
    let replacement = WordBox { rect: [80.0, 700.0, 100.0, 716.0], text: "Vendredi".into() };
    let neighbour = WordBox { rect: [150.0, 700.0, 210.0, 716.0], text: "avril".into() };

    let (mut doc, page_id) = blank_page();
    // Draw the neighbour as normal scanned-like text first (its own paint).
    paint_line(&mut doc, page_id, &geometry, std::slice::from_ref(&neighbour), None, [1.0, 1.0, 1.0], PaintFont::Embedded(&font)).unwrap();
    // Now the reflowed correction that pushes it.
    paint_line(&mut doc, page_id, &geometry, &[replacement, neighbour.clone()], Some(210.0), [1.0, 1.0, 1.0], PaintFont::Embedded(&font)).unwrap();

    // Render and scan each column: there must be a blank gap between the two
    // words, i.e. a fully white vertical strip somewhere between them.
    let path = std::env::temp_dir().join("pdf_editor_reflow.pdf");
    doc.save(&path).unwrap();
    let bytes = pdf_editor::pdf::render::render_page_to_png_bytes(&path, 1, DPI).unwrap();
    let img = image::load_from_memory(&bytes).unwrap().to_luma8();

    let y0 = ((792.0 - 716.0) * DPI as f32 / 72.0) as u32;
    let y1 = ((792.0 - 700.0) * DPI as f32 / 72.0) as u32;
    let mut columns_with_ink: Vec<bool> = Vec::new();
    for x in 0..img.width() {
        let mut ink = false;
        for y in y0..y1.min(img.height()) {
            if img.get_pixel(x, y)[0] < 128 { ink = true; break; }
        }
        columns_with_ink.push(ink);
    }
    // Count separate ink groups on the line: reflow must keep at least two words,
    // i.e. at least one blank column between ink runs.
    let groups = columns_with_ink
        .windows(2)
        .filter(|w| !w[0] && w[1])
        .count()
        + if columns_with_ink.first() == Some(&true) { 1 } else { 0 };
    assert!(groups >= 2, "les mots se chevauchent : {groups} groupe(s) d'encre");
}
