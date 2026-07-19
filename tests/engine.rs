// =============================================================================
// PDF Lks Text Editor - Tests : Moteur PDF (extraction, édition, rendu)
// Créé par Leksa667 (https://github.com/Leksa667)
// =============================================================================

use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document, Object, Stream, StringFormat};
use pdf_editor::pdf::document::PageGeometry;
use pdf_editor::pdf::edit::set_run_text;
use pdf_editor::pdf::text::extract_runs;

fn sample() -> (Document, lopdf::ObjectId) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let resources = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });
    let content = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 24.into()]),
            Operation::new("Td", vec![72.into(), 700.into()]),
            Operation::new(
                "Tj",
                vec![Object::String(b"Bonjour le monde".to_vec(), StringFormat::Literal)],
            ),
            Operation::new("ET", vec![]),
        ],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
        "Resources" => resources,
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

#[test]
fn extracts_positioned_runs() {
    let (doc, page_id) = sample();
    let runs = extract_runs(&doc, page_id).unwrap();
    assert_eq!(runs.len(), 1);
    let run = &runs[0];
    assert_eq!(run.text, "Bonjour le monde");
    assert_eq!(run.base_font, "Helvetica");
    assert!((run.font_size - 24.0).abs() < 0.01);
    assert!((run.rect[0] - 72.0).abs() < 0.5, "x0 = {}", run.rect[0]);
    assert!(run.rect[1] < 700.0 && run.rect[3] > 700.0, "baseline hors bbox");
    assert!(run.rect[2] > 200.0, "largeur trop faible : {:?}", run.rect);
    assert!(run.editable);
}

#[test]
fn edits_text_in_place_keeping_font() {
    let (mut doc, page_id) = sample();
    let run = extract_runs(&doc, page_id).unwrap().remove(0);
    set_run_text(&mut doc, page_id, run.op_index, &run.font_res, run.two_byte, "Éditeur PDF à jour").unwrap();

    let runs = extract_runs(&doc, page_id).unwrap();
    assert_eq!(runs[0].text, "Éditeur PDF à jour");
    assert_eq!(runs[0].base_font, "Helvetica");
    assert!((runs[0].font_size - 24.0).abs() < 0.01);
    assert!((runs[0].rect[0] - 72.0).abs() < 0.5);
}

#[test]
fn rejects_unencodable_characters() {
    let (mut doc, page_id) = sample();
    let run = extract_runs(&doc, page_id).unwrap().remove(0);
    let err = set_run_text(&mut doc, page_id, run.op_index, &run.font_res, run.two_byte, "漢字テスト").unwrap_err();
    assert!(err.to_string().contains("police"), "{err}");
}

#[test]
fn rendered_pixels_match_run_rectangle() {
    if !pdf_editor::pdf::render::is_renderer_available() {
        eprintln!("moteur de rendu absent, test ignoré");
        return;
    }
    use pdf_editor::pdf::document::PdfDocument;
    use pdf_editor::pdf::text::pdf_rect_to_image;

    let (mut doc, page_id) = sample();
    let path = std::env::temp_dir().join("pdf_editor_test_render.pdf");
    doc.save(&path).unwrap();

    let run = extract_runs(&doc, page_id).unwrap().remove(0);
    let pdfdoc = PdfDocument::open(&path).unwrap();
    let geom = pdfdoc.geometry(0).unwrap();
    let dpi = 150;
    let r = pdf_rect_to_image(&geom, dpi, run.rect);

    let bytes = pdf_editor::pdf::render::render_page_to_png_bytes(&path, 1, dpi).unwrap();
    let img = image::load_from_memory(&bytes).unwrap().to_luma8();

    let inside_dark = (r[0] as u32..r[2] as u32)
        .flat_map(|x| (r[1] as u32..r[3] as u32).map(move |y| (x, y)))
        .filter(|(x, y)| *x < img.width() && *y < img.height())
        .filter(|(x, y)| img.get_pixel(*x, *y)[0] < 128)
        .count();
    assert!(inside_dark > 100, "aucun texte dans le rectangle calculé ({inside_dark} px)");

    let total_dark = img.pixels().filter(|p| p[0] < 128).count();
    let ratio = inside_dark as f32 / total_dark as f32;
    assert!(ratio > 0.95, "texte hors du rectangle : {ratio:.2} du noir seulement dedans");
}

fn flat() -> PageGeometry {
    PageGeometry { x0: 0.0, y0: 0.0, width: 612.0, height: 792.0, rotate: 0 }
}

/// A scanned page: no text at all, just a grey block standing in for the scan.
fn scanned_page() -> (Document, lopdf::ObjectId) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let content = Content {
        operations: vec![
            Operation::new("q", vec![]),
            Operation::new("rg", vec![0.9.into(), 0.2.into(), 0.2.into()]),
            Operation::new("re", vec![50.into(), 600.into(), 300.into(), 40.into()]),
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
    (doc, page_id)
}

#[test]
fn scanned_region_is_masked_and_rewritten_as_real_text() {
use pdf_editor::pdf::overlay::{paint_text_region, PaintFont};

    let (mut doc, page_id) = scanned_page();
    assert!(extract_runs(&doc, page_id).unwrap().is_empty(), "la page doit être sans texte");

    let region = [50.0, 600.0, 350.0, 640.0];
    paint_text_region(&mut doc, page_id, &flat(), region, "Facture n° 2026-014", [1.0, 1.0, 1.0], PaintFont::Helvetica).unwrap();

    // The painted text is now a genuine, re-editable text run.
    let runs = extract_runs(&doc, page_id).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].text, "Facture n° 2026-014");
    assert!(runs[0].editable);
    assert!(runs[0].rect[0] >= 45.0 && runs[0].rect[2] <= 360.0, "hors zone : {:?}", runs[0].rect);

    // And it can be edited again like any other text.
    let run = runs[0].clone();
    set_run_text(&mut doc, page_id, run.op_index, &run.font_res, run.two_byte, "Facture n° 2026-015").unwrap();
    assert_eq!(extract_runs(&doc, page_id).unwrap()[0].text, "Facture n° 2026-015");
}

#[test]
fn scanned_region_mask_actually_covers_the_scan() {
    if !pdf_editor::pdf::render::is_renderer_available() {
        eprintln!("moteur de rendu absent, test ignoré");
        return;
    }
    use pdf_editor::pdf::document::PdfDocument;
use pdf_editor::pdf::overlay::{paint_text_region, PaintFont};
    use pdf_editor::pdf::text::pdf_rect_to_image;

    let dpi = 150;
    let region = [50.0, 600.0, 350.0, 640.0];

    let count = |doc: &mut Document, name: &str| -> (usize, usize) {
        let path = std::env::temp_dir().join(name);
        doc.save(&path).unwrap();
        let bytes = pdf_editor::pdf::render::render_page_to_png_bytes(&path, 1, dpi).unwrap();
        let img = image::load_from_memory(&bytes).unwrap().to_rgb8();
        let geom = PdfDocument::open(&path).unwrap().geometry(0).unwrap();
        let r = pdf_rect_to_image(&geom, dpi, region);
        let mut scan = 0;
        let mut ink = 0;
        for y in r[1] as u32..r[3] as u32 {
            for x in r[0] as u32..r[2] as u32 {
                if x >= img.width() || y >= img.height() {
                    continue;
                }
                let p = img.get_pixel(x, y);
                if p[0] > 150 && p[1] < 120 && p[2] < 120 {
                    scan += 1;
                } else if p[0] < 100 && p[1] < 100 && p[2] < 100 {
                    ink += 1;
                }
            }
        }
        (scan, ink)
    };

    let (mut doc, page_id) = scanned_page();
    let (scan_before, _) = count(&mut doc, "pdf_editor_scan_before.pdf");
    assert!(scan_before > 1000, "le faux scan n'est pas rendu");

    paint_text_region(&mut doc, page_id, &flat(), region, "Texte corrige", [1.0, 1.0, 1.0], PaintFont::Helvetica).unwrap();
    let (scan_after, ink_after) = count(&mut doc, "pdf_editor_scan_after.pdf");

    assert_eq!(scan_after, 0, "le scan n'est pas masqué ({scan_after} px restants)");
    assert!(ink_after > 200, "le texte n'a pas été peint ({ink_after} px)");
}
