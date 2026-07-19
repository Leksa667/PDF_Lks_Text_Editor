// =============================================================================
// PDF Lks Text Editor - Tests : Flux complet de l'application
// Créé par Leksa667 (https://github.com/Leksa667)
// =============================================================================

use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document, Object, Stream, StringFormat};
use pdf_editor::app::PdfEditorApp;
use pdf_editor::pdf::document::PdfDocument;
use pdf_editor::pdf::text::extract_runs;

fn write_sample(path: &std::path::Path) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let resources = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
    let content = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 24.into()]),
            Operation::new("Td", vec![72.into(), 700.into()]),
            Operation::new(
                "Tj",
                vec![Object::String(b"Montant: 100 euros".to_vec(), StringFormat::Literal)],
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
    doc.save(path).unwrap();
}

/// The whole user-facing flow: open a file, edit a block, save, and check the
/// file on disk really changed. This is the path the UI drives.
#[test]
fn editing_then_saving_changes_the_file_on_disk() {
    let source = std::env::temp_dir().join("pdf_editor_appflow.pdf");
    let saved = std::env::temp_dir().join("pdf_editor_appflow_saved.pdf");
    write_sample(&source);

    let mut app = PdfEditorApp::new();
    app.open_path(source.clone());

    assert!(app.edit_run(0, "Montant: 250 euros"), "édition refusée : {}", app.status());
    app.save_document(&saved).unwrap();

    // Re-read from disk: the edit must be in the bytes, not just in memory.
    let doc = PdfDocument::open(&saved).unwrap();
    let runs = extract_runs(&doc.doc, doc.page_id(0).unwrap()).unwrap();
    assert_eq!(runs[0].text, "Montant: 250 euros");
    assert_eq!(runs[0].base_font, "Helvetica", "la police doit être conservée");

    let bytes = std::fs::read(&saved).unwrap();
    assert!(
        !String::from_utf8_lossy(&bytes).contains("100 euros"),
        "l'ancien texte est encore dans le fichier"
    );
}

/// Enter used to insert a newline in the edit box instead of validating; the
/// newline then travelled into the encoder and killed the edit silently.
#[test]
fn a_newline_typed_into_the_box_does_not_kill_the_edit() {
    let source = std::env::temp_dir().join("pdf_editor_appflow2.pdf");
    let saved = std::env::temp_dir().join("pdf_editor_appflow2_saved.pdf");
    write_sample(&source);

    let mut app = PdfEditorApp::new();
    app.open_path(source);

    assert!(
        app.edit_run(0, "Montant: 300 euros\n"),
        "un saut de ligne ne doit pas faire échouer l'édition : {}",
        app.status()
    );
    app.save_document(&saved).unwrap();

    let doc = PdfDocument::open(&saved).unwrap();
    let runs = extract_runs(&doc.doc, doc.page_id(0).unwrap()).unwrap();
    assert_eq!(runs[0].text, "Montant: 300 euros");
}

#[test]
fn an_impossible_edit_is_reported_and_leaves_the_file_intact() {
    let source = std::env::temp_dir().join("pdf_editor_appflow3.pdf");
    write_sample(&source);

    let mut app = PdfEditorApp::new();
    app.open_path(source);

    // Helvetica has no CJK glyphs: the edit must fail loudly, not silently.
    assert!(!app.edit_run(0, "領収書"));
    assert!(
        app.status().contains("Caractères") || app.status().contains("impossible"),
        "erreur peu claire : {}",
        app.status()
    );
}

/// Builds a page holding a scan image plus `text` as a stray text layer.
fn write_scan_with_text_layer(path: &std::path::Path, text: &[u8]) {
    use lopdf::Stream;

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();

    // A tiny 2x2 image standing in for the scan.
    let image = Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => 2,
            "Height" => 2,
            "ColorSpace" => "DeviceGray",
            "BitsPerComponent" => 8,
        },
        vec![200u8, 180, 160, 140],
    );
    let image_id = doc.add_object(image);
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let resources = doc.add_object(dictionary! {
        "XObject" => dictionary! { "Im0" => image_id },
        "Font" => dictionary! { "F1" => font_id },
    });

    let content = Content {
        operations: vec![
            Operation::new("q", vec![]),
            Operation::new("cm", vec![612.into(), 0.into(), 0.into(), 792.into(), 0.into(), 0.into()]),
            Operation::new("Do", vec![Object::Name(b"Im0".to_vec())]),
            Operation::new("Q", vec![]),
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![50.into(), 50.into()]),
            Operation::new("Tj", vec![Object::String(text.to_vec(), StringFormat::Literal)]),
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
    doc.save(path).unwrap();
}

/// A scan that already carries a text layer — a stray one from the copier, or a
/// correction this editor wrote earlier — must still be OCR'd, otherwise its
/// words are not clickable and the page looks uneditable.
#[test]
fn a_scan_with_a_stray_text_layer_is_still_treated_as_a_scan() {
    let scan = std::env::temp_dir().join("pdf_editor_scan_layer.pdf");
    write_scan_with_text_layer(&scan, b"DENIS LOL");
    let doc = PdfDocument::open(&scan).unwrap();
    assert!(doc.is_scanned(0), "une image + un texte parasite = un scan");

    // A page whose text is substantial is a real text page, logo or not.
    let article = std::env::temp_dir().join("pdf_editor_article.pdf");
    let body = "Ceci est un vrai document texte avec un logo en haut de la page. ".repeat(6);
    write_scan_with_text_layer(&article, body.as_bytes());
    let doc = PdfDocument::open(&article).unwrap();
    assert!(!doc.is_scanned(0), "un article avec logo n'est pas un scan");

    // And a page with no image at all is never a scan.
    let plain = std::env::temp_dir().join("pdf_editor_plain.pdf");
    write_sample(&plain);
    let doc = PdfDocument::open(&plain).unwrap();
    assert!(!doc.is_scanned(0));
}
