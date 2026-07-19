// =============================================================================
// PDF Lks Text Editor - Tests : Polices et encodages PDF
// Créé par Leksa667 (https://github.com/Leksa667)
// =============================================================================

use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream, StringFormat};
use pdf_editor::pdf::edit::set_run_text;
use pdf_editor::pdf::font::{parse_truetype_cmap, FontEncoder, MapSource};
use pdf_editor::pdf::text::extract_runs;

/// Builds a one-page document whose single text operation uses `font`.
fn page_with_font(font: Dictionary, show: Vec<u8>, hex: bool) -> (Document, ObjectId) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(font);
    let resources = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });
    let format = if hex { StringFormat::Hexadecimal } else { StringFormat::Literal };
    let content = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 18.into()]),
            Operation::new("Td", vec![60.into(), 700.into()]),
            Operation::new("Tj", vec![Object::String(show, format)]),
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
fn custom_differences_encoding_round_trips() {
    // Code 1 = 'e' with acute, code 2 = 'A': a layout only expressible via /Differences.
    let font = dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "MyCustom+Garamond",
        "Encoding" => dictionary! {
            "Type" => "Encoding",
            "BaseEncoding" => "WinAnsiEncoding",
            "Differences" => vec![
                1.into(),
                Object::Name(b"eacute".to_vec()),
                Object::Name(b"A".to_vec()),
            ],
        },
    };
    let (mut doc, page_id) = page_with_font(font, vec![1, 2], false);

    let run = extract_runs(&doc, page_id).unwrap().remove(0);
    assert_eq!(run.text, "éA");
    assert!(run.editable);

    set_run_text(&mut doc, page_id, run.op_index, &run.font_res, run.two_byte, "AéA").unwrap();
    let runs = extract_runs(&doc, page_id).unwrap();
    assert_eq!(runs[0].text, "AéA");
}

fn to_unicode_cmap(entries: &[(u16, char)]) -> Stream {
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n\
         12 dict begin\nbegincmap\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    cmap.push_str(&format!("{} beginbfchar\n", entries.len()));
    for (code, ch) in entries {
        cmap.push_str(&format!("<{:04X}> <{:04X}>\n", code, *ch as u32));
    }
    cmap.push_str("endbfchar\nendcmap\nCMapName currentdict /CMap defineresource pop\nend\nend");
    Stream::new(dictionary! {}, cmap.into_bytes())
}

#[test]
fn cid_font_with_to_unicode_round_trips() {
    let mut doc = Document::with_version("1.5");
    let cmap_id = doc.add_object(to_unicode_cmap(&[(3, 'B'), (4, 'o'), (5, 'n'), (6, 'j')]));
    let descendant = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "CIDFontType2",
        "BaseFont" => "ABCDEF+Inter",
        "CIDSystemInfo" => dictionary! {
            "Registry" => Object::string_literal("Adobe"),
            "Ordering" => Object::string_literal("Identity"),
            "Supplement" => 0,
        },
        "DW" => 600,
    });
    let font = dictionary! {
        "Type" => "Font",
        "Subtype" => "Type0",
        "BaseFont" => "ABCDEF+Inter",
        "Encoding" => "Identity-H",
        "DescendantFonts" => vec![descendant.into()],
        "ToUnicode" => cmap_id,
    };

    // "Bonj" as 2-byte CIDs.
    let show = vec![0, 3, 0, 4, 0, 5, 0, 6];
    let font_id = doc.add_object(font);
    let resources = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
    let pages_id = doc.new_object_id();
    let content = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 20.into()]),
            Operation::new("Td", vec![50.into(), 500.into()]),
            Operation::new("Tj", vec![Object::String(show, StringFormat::Hexadecimal)]),
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

    let run = extract_runs(&doc, page_id).unwrap().remove(0);
    assert_eq!(run.text, "Bonj");
    assert!(run.two_byte && run.editable);
    assert_eq!(run.map_source, "ToUnicode");

    set_run_text(&mut doc, page_id, run.op_index, &run.font_res, run.two_byte, "Bonjonjo").unwrap();
    assert_eq!(extract_runs(&doc, page_id).unwrap()[0].text, "Bonjonjo");

    // A glyph the subset font does not contain must be refused, not corrupted.
    let run = extract_runs(&doc, page_id).unwrap().remove(0);
    let err = set_run_text(&mut doc, page_id, run.op_index, &run.font_res, run.two_byte, "Zorro")
        .unwrap_err()
        .to_string();
    assert!(err.contains('Z'), "{err}");
}

#[test]
fn typographic_variants_fall_back_to_available_glyphs() {
    let font = dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    };
    let (doc, _) = page_with_font(font.clone(), b"x".to_vec(), false);
    let encoder = FontEncoder::build(&doc, &font);

    // WinAnsi does contain the curly apostrophe and the en dash, so they survive
    // untouched; the fi ligature and the non-breaking space do not exist there and
    // must degrade to "fi" and a plain space rather than failing the edit.
    let bytes = encoder.encode("l\u{2019}\u{FB01}n \u{2013} a\u{00A0}b").unwrap();
    assert_eq!(encoder.decode(&bytes), "l\u{2019}fin \u{2013} a b");
}

#[test]
fn reads_cmap_of_a_real_system_font() {
    let path = std::path::Path::new(r"C:\Windows\Fonts\arial.ttf");
    if !path.exists() {
        eprintln!("arial.ttf absent, test ignoré");
        return;
    }
    let data = std::fs::read(path).unwrap();
    let cmap = parse_truetype_cmap(&data);
    assert!(cmap.len() > 200, "cmap trop petite : {}", cmap.len());
    assert!(cmap.contains_key(&'A'));
    assert!(cmap.contains_key(&'é'));
    assert_ne!(cmap[&'A'], cmap[&'B']);
}

#[test]
fn cid_font_without_to_unicode_uses_embedded_cmap() {
    let path = std::path::Path::new(r"C:\Windows\Fonts\arial.ttf");
    if !path.exists() {
        eprintln!("arial.ttf absent, test ignoré");
        return;
    }
    let data = std::fs::read(path).unwrap();
    let gids = parse_truetype_cmap(&data);

    let mut doc = Document::with_version("1.5");
    let file_id = doc.add_object(Stream::new(dictionary! { "Length1" => data.len() as i64 }, data));
    let descriptor = doc.add_object(dictionary! {
        "Type" => "FontDescriptor",
        "FontName" => "Arial",
        "Flags" => 4,
        "FontFile2" => file_id,
    });
    let descendant = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "CIDFontType2",
        "BaseFont" => "Arial",
        "CIDToGIDMap" => Object::Name(b"Identity".to_vec()),
        "FontDescriptor" => descriptor,
        "DW" => 600,
    });
    // No /ToUnicode at all: only the embedded cmap can save this font.
    let font = dictionary! {
        "Type" => "Font",
        "Subtype" => "Type0",
        "BaseFont" => "Arial",
        "Encoding" => "Identity-H",
        "DescendantFonts" => vec![descendant.into()],
    };

    let encoder = FontEncoder::build(&doc, &font);
    assert_eq!(encoder.source, MapSource::EmbeddedCmap);
    assert!(encoder.is_usable());

    let bytes = encoder.encode("Créé").unwrap();
    assert_eq!(bytes.len(), 8, "4 caractères sur 2 octets");
    let expected: Vec<u8> = "Créé"
        .chars()
        .flat_map(|c| {
            let gid = gids[&c];
            [(gid >> 8) as u8, gid as u8]
        })
        .collect();
    assert_eq!(bytes, expected);
    assert_eq!(encoder.decode(&bytes), "Créé");
}
