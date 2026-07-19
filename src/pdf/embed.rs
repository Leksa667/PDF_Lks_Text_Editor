// =============================================================================
// PDF Lks Text Editor - Éditeur PDF avec OCR et édition de texte
// Créé par Leksa667 (https://github.com/Leksa667)
//
// Ce logiciel est en libre service. Vous pouvez l'utiliser, le modifier
// et le distribuer librement, à condition de créditer l'auteur original.
// Aucune garantie n'est fournie.
// =============================================================================

use crate::pdf::font::FontEncoder;
use crate::pdf::subset::{base_charset, subset};
use crate::pdf::ttf::TtfFont;
use anyhow::{anyhow, Result};
use lopdf::{dictionary, Document, Object, ObjectId, Stream};
use std::collections::{BTreeSet, HashMap};

/// A TrueType font written into the document as a CIDFontType2 / Identity-H
/// composite font, with a generated /ToUnicode CMap so the text stays
/// extractable, searchable and re-editable afterwards.
pub struct EmbeddedFont {
    pub resource: String,
    pub font_id: ObjectId,
}

fn build_to_unicode(font: &TtfFont, chars: &BTreeSet<char>, gid_map: &HashMap<u16, u16>) -> Stream {
    let mut pairs: Vec<(u16, char)> = chars
        .iter()
        .filter_map(|c| font.gid(*c).and_then(|g| gid_map.get(&g)).map(|g| (*g, *c)))
        .collect();
    pairs.sort_unstable();
    pairs.dedup_by_key(|(gid, _)| *gid);

    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n\
         12 dict begin\n\
         begincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n\
         /CMapType 2 def\n\
         1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );

    for chunk in pairs.chunks(100) {
        cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (gid, ch) in chunk {
            let mut buf = [0u16; 2];
            let units = ch.encode_utf16(&mut buf);
            let hex: String = units.iter().map(|u| format!("{u:04X}")).collect();
            cmap.push_str(&format!("<{gid:04X}> <{hex}>\n"));
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend");

    Stream::new(dictionary! {}, cmap.into_bytes())
}

/// /W array for the embedded glyphs only, grouped into runs of consecutive ids
/// (`first [w1 w2 …]`) so the array stays small.
fn build_widths(font: &TtfFont, gid_map: &HashMap<u16, u16>) -> Vec<Object> {
    let mut subset_gids: Vec<(u16, u16)> = gid_map.iter().map(|(old, new)| (*new, *old)).collect();
    subset_gids.sort_unstable();

    let mut out = Vec::new();
    let mut run: Vec<Object> = Vec::new();
    let mut run_start = 0u16;
    let mut previous: Option<u16> = None;

    for (gid, original) in subset_gids {
        match previous {
            Some(p) if gid == p + 1 => {}
            _ => {
                if !run.is_empty() {
                    out.push(Object::Integer(run_start as i64));
                    out.push(Object::Array(std::mem::take(&mut run)));
                }
                run_start = gid;
            }
        }
        run.push(Object::Integer(font.advance(original) as i64));
        previous = Some(gid);
    }
    if !run.is_empty() {
        out.push(Object::Integer(run_start as i64));
        out.push(Object::Array(run));
    }
    out
}

/// Does the font already sitting under `resource` cover every character we are
/// about to draw? A subset embedded for an earlier edit may not.
fn covers(doc: &Document, page_id: ObjectId, resource: &str, text: &str) -> bool {
    let Ok(fonts) = doc.get_page_fonts(page_id) else { return false };
    let Some(dict) = fonts.get(resource.as_bytes()) else { return false };
    FontEncoder::build(doc, dict).encode(text).is_ok()
}

fn resources_id(doc: &mut Document, page_id: ObjectId) -> Result<ObjectId> {
    let mut current = page_id;
    for _ in 0..32 {
        let dict = doc
            .get_dictionary(current)
            .map_err(|e| anyhow!("Page illisible : {e}"))?;
        match dict.get(b"Resources") {
            Ok(Object::Reference(id)) => return Ok(*id),
            Ok(Object::Dictionary(d)) => {
                let d = d.clone();
                let id = doc.add_object(d);
                doc.get_dictionary_mut(current)
                    .map_err(|e| anyhow!("Page illisible : {e}"))?
                    .set("Resources", id);
                return Ok(id);
            }
            _ => {}
        }
        match dict.get(b"Parent").and_then(|p| p.as_reference()) {
            Ok(parent) => current = parent,
            Err(_) => break,
        }
    }
    let id = doc.add_object(lopdf::Dictionary::new());
    doc.get_dictionary_mut(page_id)
        .map_err(|e| anyhow!("Page illisible : {e}"))?
        .set("Resources", id);
    Ok(id)
}

fn set_page_font(doc: &mut Document, page_id: ObjectId, name: &str, font_id: ObjectId) -> Result<()> {
    let resources_id = resources_id(doc, page_id)?;
    let entry = doc
        .get_dictionary(resources_id)
        .ok()
        .and_then(|res| res.get(b"Font").ok().cloned());

    match entry {
        Some(Object::Reference(id)) => {
            doc.get_dictionary_mut(id)
                .map_err(|e| anyhow!("Ressources de police illisibles : {e}"))?
                .set(name, font_id);
        }
        Some(Object::Dictionary(mut fonts)) => {
            fonts.set(name, font_id);
            doc.get_dictionary_mut(resources_id)
                .map_err(|e| anyhow!("Ressources illisibles : {e}"))?
                .set("Font", fonts);
        }
        _ => {
            let fonts = dictionary! { name => font_id };
            doc.get_dictionary_mut(resources_id)
                .map_err(|e| anyhow!("Ressources illisibles : {e}"))?
                .set("Font", fonts);
        }
    }
    Ok(())
}

fn existing_font(doc: &Document, page_id: ObjectId, name: &str) -> Option<ObjectId> {
    let fonts = doc.get_page_fonts(page_id).ok()?;
    if !fonts.contains_key(name.as_bytes()) {
        return None;
    }
    // get_page_fonts hands back dictionaries, not ids: walk the resources again.
    let resources = doc.get_dictionary(page_id).ok()?.get(b"Resources").ok()?;
    let resources = match resources {
        Object::Reference(id) => doc.get_dictionary(*id).ok()?,
        Object::Dictionary(d) => d,
        _ => return None,
    };
    let font_dict = match resources.get(b"Font").ok()? {
        Object::Reference(id) => doc.get_dictionary(*id).ok()?,
        Object::Dictionary(d) => d,
        _ => return None,
    };
    font_dict.get(name.as_bytes()).ok()?.as_reference().ok()
}

/// Writes a subset of `font` into the document, keeping only the glyphs needed
/// for `text` (plus a Latin base set), and returns the resource name to
/// reference it from a content stream. The subsetter renumbers glyphs, so the
/// /W array and /ToUnicode CMap written here use the subset's ids — encode text
/// for this font with [`encode_for_resource`], never with the original face.
pub fn embed_font(
    doc: &mut Document,
    page_id: ObjectId,
    font: &TtfFont,
    text: &str,
) -> Result<EmbeddedFont> {
    let base = format!("LksTT{}", font.pdf_name());

    // Reuse an identical face already on the page, but only if its subset holds
    // every character of this edit; otherwise embed a second, wider subset.
    let mut resource = base.clone();
    let mut suffix = 1;
    while let Some(font_id) = existing_font(doc, page_id, &resource) {
        if covers(doc, page_id, &resource, text) {
            return Ok(EmbeddedFont { resource, font_id });
        }
        suffix += 1;
        resource = format!("{base}{suffix}");
    }

    let mut chars = base_charset();
    chars.extend(text.chars());
    chars.retain(|c| font.gid(*c).is_some());

    let subsetted = subset(font, &chars)?;
    let length1 = subsetted.program.len() as i64;
    let mut file_stream = Stream::new(dictionary! { "Length1" => length1 }, subsetted.program);
    let _ = file_stream.compress();
    let file_id = doc.add_object(file_stream);

    let descriptor_id = doc.add_object(dictionary! {
        "Type" => "FontDescriptor",
        "FontName" => Object::Name(font.pdf_name().into_bytes()),
        "Flags" => font.flags,
        "FontBBox" => vec![
            font.bbox[0].into(),
            font.bbox[1].into(),
            font.bbox[2].into(),
            font.bbox[3].into(),
        ],
        "ItalicAngle" => font.italic_angle as f64,
        "Ascent" => font.ascent as f64,
        "Descent" => font.descent as f64,
        "CapHeight" => font.cap_height as f64,
        "StemV" => 80,
        "FontFile2" => file_id,
    });

    let descendant_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "CIDFontType2",
        "BaseFont" => Object::Name(font.pdf_name().into_bytes()),
        "CIDSystemInfo" => dictionary! {
            "Registry" => Object::string_literal("Adobe"),
            "Ordering" => Object::string_literal("Identity"),
            "Supplement" => 0,
        },
        "FontDescriptor" => descriptor_id,
        "DW" => 1000,
        "W" => build_widths(font, &subsetted.gid_map),
        "CIDToGIDMap" => Object::Name(b"Identity".to_vec()),
    });

    let to_unicode_id = doc.add_object(build_to_unicode(font, &chars, &subsetted.gid_map));

    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type0",
        "BaseFont" => Object::Name(font.pdf_name().into_bytes()),
        "Encoding" => "Identity-H",
        "DescendantFonts" => vec![descendant_id.into()],
        "ToUnicode" => to_unicode_id,
    });

    set_page_font(doc, page_id, &resource, font_id)?;
    Ok(EmbeddedFont { resource, font_id })
}

/// Encodes `text` for a font already embedded on the page, reading the glyph
/// ids straight back out of the PDF. This is the only safe source of truth: the
/// subsetter renumbers glyphs, so the original font's ids no longer apply.
pub fn encode_for_resource(
    doc: &Document,
    page_id: ObjectId,
    resource: &str,
    text: &str,
) -> Result<Vec<u8>> {
    let fonts = doc
        .get_page_fonts(page_id)
        .map_err(|e| anyhow!("Polices de la page illisibles : {e}"))?;
    let dict = fonts
        .get(resource.as_bytes())
        .ok_or_else(|| anyhow!("Police /{resource} absente de la page"))?;

    FontEncoder::build(doc, dict)
        .encode(text)
        .map_err(|missing| {
            anyhow!(
                "Caractères absents de la police : {}",
                missing.chars().take(12).collect::<String>()
            )
        })
}
