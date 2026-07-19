// =============================================================================
// PDF Lks Text Editor - Éditeur PDF avec OCR et édition de texte
// Créé par Leksa667 (https://github.com/Leksa667)
//
// Ce logiciel est en libre service. Vous pouvez l'utiliser, le modifier
// et le distribuer librement, à condition de créditer l'auteur original.
// Aucune garantie n'est fournie.
// =============================================================================

use crate::pdf::document::PageGeometry;
use crate::pdf::embed::{embed_font, encode_for_resource};
use crate::pdf::font::FontEncoder;
use crate::pdf::ttf::TtfFont;
use anyhow::{anyhow, Result};
use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, StringFormat};

const HELVETICA_RESOURCE: &str = "LksHelv";
const HELVETICA_ASCENT: f32 = 718.0;
const HELVETICA_DESCENT: f32 = -207.0;

/// Helvetica AFM advance widths (1/1000 em) for the printable ASCII range.
const HELVETICA_WIDTHS: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667,
    611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500,
    222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];

fn helvetica_width(c: char) -> f32 {
    let code = c as u32;
    if (32..127).contains(&code) {
        HELVETICA_WIDTHS[(code - 32) as usize] as f32
    } else {
        556.0
    }
}

/// Which font to repaint a scanned region with.
pub enum PaintFont<'a> {
    /// Non-embedded standard font: always available, but not the scan's font.
    Helvetica,
    /// A real TrueType face, embedded into the document.
    Embedded(&'a TtfFont),
}

struct Painted {
    resource: String,
    bytes: Vec<u8>,
    hex: bool,
    natural_width: f32,
    left_bearing: f32,
    size: f32,
    baseline_offset: f32,
}

impl PaintFont<'_> {
    /// Sizes the text so its ascent-to-descent band fills the scanned box, then
    /// reports how wide it naturally is so the caller can stretch it to fit.
    fn prepare(&self, doc: &mut Document, page_id: ObjectId, text: &str, height: f32) -> Result<Painted> {
        match self {
            PaintFont::Helvetica => {
                ensure_helvetica(doc, page_id)?;
                let size = height * 1000.0 / (HELVETICA_ASCENT - HELVETICA_DESCENT);
                let bytes = winansi_encode(doc, text)?;
                let natural_width: f32 = text.chars().map(|c| helvetica_width(c) / 1000.0 * size).sum();
                Ok(Painted {
                    resource: HELVETICA_RESOURCE.to_string(),
                    bytes,
                    hex: false,
                    natural_width,
                    left_bearing: 0.0,
                    size,
                    baseline_offset: -HELVETICA_DESCENT / 1000.0 * size,
                })
            }
            PaintFont::Embedded(font) => {
                let embedded = embed_font(doc, page_id, font, text)?;
                let bytes = encode_for_resource(doc, page_id, &embedded.resource, text)?;

                // The OCR box hugs the ink of *this* word: a word with neither
                // ascender nor descender yields a short box, and its first glyph
                // has a left bearing. Sizing from ascent/descent and starting the
                // pen at the box edge would misplace the text on both axes.
                let (size, ink_width, left_bearing, baseline_offset) = fit_to_box(font, text, height);

                Ok(Painted {
                    resource: embedded.resource,
                    bytes,
                    hex: true,
                    natural_width: ink_width,
                    left_bearing,
                    size,
                    baseline_offset,
                })
            }
        }
    }
}

/// The box actually covered by ink when `text` is set in `font`, in em units
/// relative to the pen origin on the baseline. This is what an OCR bounding box
/// measures — not the ascent/descent band, and not the advance width.
#[derive(Clone, Copy, Debug)]
pub struct InkBox {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

pub fn ink_box(font: &TtfFont, text: &str) -> Option<InkBox> {
    const REF: f32 = 100.0;
    let rasterizer =
        fontdue::Font::from_bytes(font.data.as_slice(), fontdue::FontSettings::default()).ok()?;

    let mut pen = 0.0f32;
    let (mut left, mut right, mut top, mut bottom) = (f32::MAX, f32::MIN, f32::MIN, f32::MAX);

    for c in text.chars() {
        let metrics = rasterizer.metrics(c, REF);
        if metrics.width > 0 && metrics.height > 0 {
            let x = pen + metrics.xmin as f32;
            left = left.min(x);
            right = right.max(x + metrics.width as f32);
            bottom = bottom.min(metrics.ymin as f32);
            top = top.max(metrics.ymin as f32 + metrics.height as f32);
        }
        pen += metrics.advance_width;
    }

    if left == f32::MAX || right <= left || top <= bottom {
        return None;
    }
    Some(InkBox {
        left: left / REF,
        right: right / REF,
        top: top / REF,
        bottom: bottom / REF,
    })
}

/// Font size and pen placement that make `text` fill a box `height` tall.
pub fn fit_to_box(font: &TtfFont, text: &str, height: f32) -> (f32, f32, f32, f32) {
    match ink_box(font, text) {
        Some(ink) => {
            let size = height / (ink.top - ink.bottom);
            (
                size,
                (ink.right - ink.left) * size, // ink width
                ink.left * size,               // left bearing
                -ink.bottom * size,            // baseline above the box bottom
            )
        }
        None => {
            let span = (font.ascent - font.descent).max(1.0);
            let size = height * 1000.0 / span;
            (size, font.text_width(text, size), 0.0, -font.descent / 1000.0 * size)
        }
    }
}

fn ensure_helvetica(doc: &mut Document, page_id: ObjectId) -> Result<()> {
    let resources_id = resources_id(doc, page_id)?;

    let fonts_entry = doc
        .get_dictionary(resources_id)
        .ok()
        .and_then(|res| res.get(b"Font").ok().cloned());

    let already = match &fonts_entry {
        Some(Object::Reference(id)) => doc
            .get_dictionary(*id)
            .map(|d| d.has(HELVETICA_RESOURCE.as_bytes()))
            .unwrap_or(false),
        Some(Object::Dictionary(d)) => d.has(HELVETICA_RESOURCE.as_bytes()),
        _ => false,
    };
    if already {
        return Ok(());
    }

    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });

    match fonts_entry {
        Some(Object::Reference(id)) => {
            doc.get_dictionary_mut(id)
                .map_err(|e| anyhow!("Ressources de police illisibles : {e}"))?
                .set(HELVETICA_RESOURCE, font_id);
        }
        Some(Object::Dictionary(mut fonts)) => {
            fonts.set(HELVETICA_RESOURCE, font_id);
            doc.get_dictionary_mut(resources_id)
                .map_err(|e| anyhow!("Ressources illisibles : {e}"))?
                .set("Font", fonts);
        }
        _ => {
            let fonts = dictionary! { HELVETICA_RESOURCE => font_id };
            doc.get_dictionary_mut(resources_id)
                .map_err(|e| anyhow!("Ressources illisibles : {e}"))?
                .set("Font", fonts);
        }
    }
    Ok(())
}

/// Resolves the object holding the page's /Resources, creating one on the page
/// itself when the dictionary is inline or inherited.
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

    let id = doc.add_object(Dictionary::new());
    doc.get_dictionary_mut(page_id)
        .map_err(|e| anyhow!("Page illisible : {e}"))?
        .set("Resources", id);
    Ok(id)
}

fn winansi_encode(doc: &Document, text: &str) -> Result<Vec<u8>> {
    let font = dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    };
    let encoder = FontEncoder::build(doc, &font);
    encoder.encode(text).map_err(|missing| {
        anyhow!(
            "Caractères non supportés par Helvetica : {}",
            missing.chars().take(12).collect::<String>()
        )
    })
}

/// Size of the page as the reader sees it, in points.
pub fn visual_size(g: &PageGeometry) -> (f32, f32) {
    match g.rotate {
        90 | 270 => (g.height, g.width),
        _ => (g.width, g.height),
    }
}

/// Visual space (origin bottom-left of the page *as displayed*) back to PDF user
/// space. /Rotate only turns the page for the reader: the content stream still
/// draws in unrotated user space, so text painted there would come out sideways
/// unless we undo the rotation ourselves.
pub fn visual_to_user(g: &PageGeometry, vx: f32, vy: f32) -> (f32, f32) {
    match g.rotate {
        90 => (g.x0 + g.width - vy, g.y0 + vx),
        180 => (g.x0 + g.width - vx, g.y0 + g.height - vy),
        270 => (g.x0 + vy, g.y0 + g.height - vx),
        _ => (g.x0 + vx, g.y0 + vy),
    }
}

/// Text matrix that makes glyphs read horizontally once the page is rotated for
/// display: the text runs along the visual +x axis.
fn text_axes(rotate: i64) -> (f32, f32, f32, f32) {
    match rotate {
        90 => (0.0, 1.0, -1.0, 0.0),
        180 => (-1.0, 0.0, 0.0, -1.0),
        270 => (0.0, -1.0, 1.0, 0.0),
        _ => (1.0, 0.0, 0.0, 1.0),
    }
}

/// Paints `text` over a rectangle of a scanned page: the region is covered with
/// `background` and the text is redrawn with `font`, scaled to fit the box.
/// `rect` is x0, y0, x1, y1 in *visual* points — the page as the reader sees it,
/// rotation included.
pub fn paint_text_region(
    doc: &mut Document,
    page_id: ObjectId,
    geometry: &PageGeometry,
    rect: [f32; 4],
    text: &str,
    background: [f32; 3],
    font: PaintFont,
) -> Result<()> {
    let word = WordBox { rect, text: text.to_string() };
    paint_line(doc, page_id, geometry, &[word], None, background, font)
}

/// One word to repaint, and the visual box the scan gave it.
#[derive(Clone)]
pub struct WordBox {
    /// x0, y0, x1, y1 in visual points.
    pub rect: [f32; 4],
    pub text: String,
}

/// Repaints a run of words on one scanned line, sharing a single mask.
///
/// `mask_right` optionally extends the covered area rightward — up to the next
/// untouched word — so a replacement longer than the original may grow into the
/// blank space after it instead of being squeezed. When several words are given,
/// they are laid out left to right keeping the original gaps, which is how a
/// following word gets pushed aside to make room.
pub fn paint_line(
    doc: &mut Document,
    page_id: ObjectId,
    geometry: &PageGeometry,
    words: &[WordBox],
    mask_right: Option<f32>,
    background: [f32; 3],
    font: PaintFont,
) -> Result<()> {
    let first = words.first().ok_or_else(|| anyhow!("Aucun mot"))?;
    let vx0 = first.rect[0].min(first.rect[2]);
    let mut vy_top = f32::MAX;
    let mut vy_bot = f32::MIN;
    let mut natural_right = vx0;
    for w in words {
        vy_top = vy_top.min(w.rect[1].min(w.rect[3]));
        vy_bot = vy_bot.max(w.rect[1].max(w.rect[3]));
        natural_right = natural_right.max(w.rect[0].max(w.rect[2]));
    }
    let height = vy_bot - vy_top;
    if height <= 0.0 {
        return Err(anyhow!("Ligne vide"));
    }

    let (a, b, c, d) = text_axes(geometry.rotate);
    let pad = height * 0.12;

    // Lay every word out, left to right, preserving the original inter-word gap.
    let mut ops: Vec<Operation> = Vec::new();
    let mut pen_cursor = vx0;
    let mut painted_right = vx0;
    let mut prev_right = vx0;

    for (i, w) in words.iter().enumerate() {
        let box_left = w.rect[0].min(w.rect[2]);
        let box_right = w.rect[0].max(w.rect[2]);
        let box_width = box_right - box_left;

        // Keep the whitespace that preceded this word on the scan.
        if i > 0 {
            let gap = (box_left - prev_right).max(0.0);
            pen_cursor += gap;
        }
        prev_right = box_right;

        // Size each word to its *own* box, not the line's tallest: a date keeps
        // its small figures, a word with ascenders keeps its height. Only then do
        // reflowed neighbours match the surrounding scan instead of ballooning.
        // Visual y points up, so the smaller edge is the box bottom.
        let box_bottom = w.rect[1].min(w.rect[3]);
        let box_height = (w.rect[3] - w.rect[1]).abs();

        let painted = font.prepare(doc, page_id, &w.text, box_height)?;

        // Only the first word is anchored to its own box; the rest flow after the
        // previous one. Room available before the next kept word (or the mask
        // limit) bounds any stretch.
        let room = if i == 0 {
            box_width
        } else {
            (natural_right - pen_cursor).max(box_width)
        };
        let h_scale = if painted.natural_width > 0.0 {
            (room / painted.natural_width * 100.0).clamp(70.0, 112.0)
        } else {
            100.0
        };

        let drawn_width = painted.natural_width * h_scale / 100.0;
        let pen_vx = pen_cursor - painted.left_bearing * h_scale / 100.0;
        let pen_vy = box_bottom + painted.baseline_offset;
        let (pen_x, pen_y) = visual_to_user(geometry, pen_vx, pen_vy);

        let string = Object::String(
            painted.bytes,
            if painted.hex { StringFormat::Hexadecimal } else { StringFormat::Literal },
        );

        ops.push(Operation::new("BT", vec![]));
        ops.push(Operation::new("rg", vec![0.into(), 0.into(), 0.into()]));
        ops.push(Operation::new(
            "Tf",
            vec![Object::Name(painted.resource.into_bytes()), painted.size.into()],
        ));
        ops.push(Operation::new("Tz", vec![h_scale.into()]));
        ops.push(Operation::new(
            "Tm",
            vec![a.into(), b.into(), c.into(), d.into(), pen_x.into(), pen_y.into()],
        ));
        ops.push(Operation::new("Tj", vec![string]));
        ops.push(Operation::new("ET", vec![]));

        pen_cursor += drawn_width;
        painted_right = painted_right.max(pen_cursor);
    }

    // Mask from the first word's left to whichever is furthest: the painted text,
    // the words' own boxes, or the caller-supplied right limit.
    let vx1 = painted_right.max(natural_right).max(mask_right.unwrap_or(0.0));
    let corner_a = visual_to_user(geometry, vx0 - pad, vy_top - pad);
    let corner_b = visual_to_user(geometry, vx1 + pad, vy_bot + pad);
    let mask_x = corner_a.0.min(corner_b.0);
    let mask_y = corner_a.1.min(corner_b.1);
    let mask_w = (corner_b.0 - corner_a.0).abs();
    let mask_h = (corner_b.1 - corner_a.1).abs();

    let mut content = vec![
        Operation::new("q", vec![]),
        Operation::new(
            "rg",
            vec![background[0].into(), background[1].into(), background[2].into()],
        ),
        Operation::new(
            "re",
            vec![mask_x.into(), mask_y.into(), mask_w.into(), mask_h.into()],
        ),
        Operation::new("f", vec![]),
    ];
    content.append(&mut ops);
    content.push(Operation::new("Q", vec![]));

    append_isolated(doc, page_id, Content { operations: content })
}

/// Appends `content` to a page after fencing the existing content inside `q`/`Q`.
///
/// Page content streams are concatenated into a single stream, so whatever
/// graphics state the original leaves behind — a scanner's unbalanced
/// `0.36 0 0 0.36 0 0 cm`, a clipping path, a fill colour — would otherwise
/// apply to everything we draw, scaling and displacing it. Wrapping the original
/// restores a clean state without altering how it renders.
pub(crate) fn append_isolated(doc: &mut Document, page_id: ObjectId, content: Content) -> Result<()> {
    let addition = content
        .encode()
        .map_err(|e| anyhow!("Encodage du contenu impossible : {e}"))?;

    let original = doc.get_page_content(page_id);
    let mut merged = Vec::with_capacity(original.len() + addition.len() + 8);
    merged.extend_from_slice(b"q\n");
    merged.extend_from_slice(&original);
    merged.extend_from_slice(b"\nQ\n");
    merged.extend_from_slice(&addition);

    doc.change_page_content(page_id, merged)
        .map_err(|e| anyhow!("Écriture du contenu impossible : {e}"))?;
    Ok(())
}
