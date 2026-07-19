// =============================================================================
// PDF Lks Text Editor - Éditeur PDF avec OCR et édition de texte
// Créé par Leksa667 (https://github.com/Leksa667)
//
// Ce logiciel est en libre service. Vous pouvez l'utiliser, le modifier
// et le distribuer librement, à condition de créditer l'auteur original.
// Aucune garantie n'est fournie.
// =============================================================================

use crate::pdf::document::{descendant_or_self, name_of, number};
use anyhow::{anyhow, Result};
use lopdf::content::Content;
use crate::pdf::font::FontEncoder;
use lopdf::{Dictionary, Document, Object, ObjectId};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
pub struct Mat {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Mat {
    pub const ID: Mat = Mat { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    fn translate(tx: f32, ty: f32) -> Mat {
        Mat { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: tx, f: ty }
    }

    /// self then other (row-vector convention: p * self * other)
    fn then(&self, o: &Mat) -> Mat {
        Mat {
            a: self.a * o.a + self.b * o.c,
            b: self.a * o.b + self.b * o.d,
            c: self.c * o.a + self.d * o.c,
            d: self.c * o.b + self.d * o.d,
            e: self.e * o.a + self.f * o.c + o.e,
            f: self.e * o.b + self.f * o.d + o.f,
        }
    }

    fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
    }
}

/// A single show-text operation, positioned in PDF user space.
#[derive(Clone, Debug)]
pub struct TextRun {
    pub op_index: usize,
    pub text: String,
    pub font_res: String,
    pub base_font: String,
    pub font_size: f32,
    /// x0, y0, x1, y1 in PDF user space (y up)
    pub rect: [f32; 4],
    pub two_byte: bool,
    pub editable: bool,
    pub map_source: &'static str,
}

struct FontMetrics {
    two_byte: bool,
    widths: HashMap<u32, f32>,
    default_width: f32,
    ascent: f32,
    descent: f32,
    base_font: String,
}

impl FontMetrics {
    fn width(&self, code: u32) -> f32 {
        *self.widths.get(&code).unwrap_or(&self.default_width)
    }

}

fn build_metrics(doc: &Document, font: &Dictionary) -> FontMetrics {
    let subtype = name_of(font, b"Subtype").unwrap_or_default();
    let base_font = name_of(font, b"BaseFont").unwrap_or_else(|| "Inconnue".into());
    let two_byte = subtype == "Type0";

    let mut widths = HashMap::new();
    let mut default_width = if two_byte { 1000.0 } else { 500.0 };

    if two_byte {
        if let Some(desc) = descendant_or_self(doc, font) {
            if let Some(dw) = desc.get(b"DW").ok().and_then(number) {
                default_width = dw;
            }
            if let Ok(obj) = desc.get(b"W") {
                if let Ok((_, Object::Array(w))) = doc.dereference(obj) {
                    parse_cid_widths(doc, w, &mut widths);
                }
            }
        }
    } else {
        let first = font.get(b"FirstChar").ok().and_then(number).unwrap_or(0.0) as i64;
        if let Ok(obj) = font.get(b"Widths") {
            if let Ok((_, Object::Array(arr))) = doc.dereference(obj) {
                for (i, w) in arr.iter().enumerate() {
                    if let Some(v) = doc.dereference(w).ok().and_then(|(_, o)| number(o)) {
                        widths.insert((first + i as i64).max(0) as u32, v);
                    }
                }
            }
        }
        if widths.is_empty() {
            for c in 32..127u32 {
                widths.insert(c, standard_width(&base_font, c));
            }
        }
    }

    let descriptor = descendant_or_self(doc, font)
        .and_then(|d| doc.get_dict_in_dict(d, b"FontDescriptor").ok());

    let ascent = descriptor
        .and_then(|d| d.get(b"Ascent").ok().and_then(number))
        .filter(|v| *v > 0.0)
        .map(|v| v / 1000.0)
        .unwrap_or(0.78);
    let descent = descriptor
        .and_then(|d| d.get(b"Descent").ok().and_then(number))
        .filter(|v| *v < 0.0)
        .map(|v| v / 1000.0)
        .unwrap_or(-0.22);

    FontMetrics {
        two_byte,
        widths,
        default_width,
        ascent,
        descent,
        base_font,
    }
}

fn parse_cid_widths(doc: &Document, w: &[Object], widths: &mut HashMap<u32, f32>) {
    let mut i = 0;
    while i < w.len() {
        let first = match doc.dereference(&w[i]).ok().and_then(|(_, o)| number(o)) {
            Some(v) => v as u32,
            None => break,
        };
        if i + 1 >= w.len() {
            break;
        }
        match doc.dereference(&w[i + 1]) {
            Ok((_, Object::Array(list))) => {
                for (k, item) in list.iter().enumerate() {
                    if let Some(v) = doc.dereference(item).ok().and_then(|(_, o)| number(o)) {
                        widths.insert(first + k as u32, v);
                    }
                }
                i += 2;
            }
            Ok((_, obj)) => {
                let last = match number(obj) {
                    Some(v) => v as u32,
                    None => break,
                };
                if i + 2 >= w.len() {
                    break;
                }
                let width = doc
                    .dereference(&w[i + 2])
                    .ok()
                    .and_then(|(_, o)| number(o))
                    .unwrap_or(1000.0);
                for c in first..=last.min(first + 65535) {
                    widths.insert(c, width);
                }
                i += 3;
            }
            Err(_) => break,
        }
    }
}

fn standard_width(base_font: &str, code: u32) -> f32 {
    let lower = base_font.to_ascii_lowercase();
    if lower.contains("courier") || lower.contains("mono") {
        return 600.0;
    }
    let ch = char::from_u32(code).unwrap_or(' ');
    match ch {
        ' ' | 'i' | 'l' | 'j' | '.' | ',' | '\'' | '!' | '|' | ':' | ';' => 278.0,
        'm' | 'w' | 'M' | 'W' => 889.0,
        'A'..='Z' => 667.0,
        _ => 500.0,
    }
}

struct TextState {
    tm: Mat,
    tlm: Mat,
    leading: f32,
    char_spacing: f32,
    word_spacing: f32,
    h_scale: f32,
    rise: f32,
    font: Option<String>,
    size: f32,
}

impl TextState {
    fn new() -> Self {
        Self {
            tm: Mat::ID,
            tlm: Mat::ID,
            leading: 0.0,
            char_spacing: 0.0,
            word_spacing: 0.0,
            h_scale: 1.0,
            rise: 0.0,
            font: None,
            size: 0.0,
        }
    }
}

fn op_number(operands: &[Object], i: usize) -> f32 {
    operands.get(i).and_then(number).unwrap_or(0.0)
}

pub fn extract_runs(doc: &Document, page_id: ObjectId) -> Result<Vec<TextRun>> {
    let page_fonts = doc.get_page_fonts(page_id).unwrap_or_default();

    let mut metrics: HashMap<String, FontMetrics> = HashMap::new();
    let mut encoders: HashMap<String, FontEncoder> = HashMap::new();
    for (key, dict) in &page_fonts {
        let name = String::from_utf8_lossy(key).to_string();
        metrics.insert(name.clone(), build_metrics(doc, dict));
        encoders.insert(name, FontEncoder::build(doc, dict));
    }

    let data = doc.get_page_content(page_id);
    let content = Content::decode(&data).map_err(|e| anyhow!("Flux de contenu illisible : {e}"))?;

    let mut runs = Vec::new();
    let mut ctm = Mat::ID;
    let mut stack: Vec<Mat> = Vec::new();
    let mut ts = TextState::new();

    for (op_index, op) in content.operations.iter().enumerate() {
        let ops = &op.operands;
        match op.operator.as_str() {
            "q" => stack.push(ctm),
            "Q" => ctm = stack.pop().unwrap_or(Mat::ID),
            "cm" => {
                let m = Mat {
                    a: op_number(ops, 0),
                    b: op_number(ops, 1),
                    c: op_number(ops, 2),
                    d: op_number(ops, 3),
                    e: op_number(ops, 4),
                    f: op_number(ops, 5),
                };
                ctm = m.then(&ctm);
            }
            "BT" => {
                ts.tm = Mat::ID;
                ts.tlm = Mat::ID;
            }
            "ET" => {}
            "Tf" => {
                ts.font = ops
                    .first()
                    .and_then(|o| o.as_name().ok())
                    .map(|n| String::from_utf8_lossy(n).to_string());
                ts.size = op_number(ops, 1);
            }
            "Td" => {
                ts.tlm = Mat::translate(op_number(ops, 0), op_number(ops, 1)).then(&ts.tlm);
                ts.tm = ts.tlm;
            }
            "TD" => {
                ts.leading = -op_number(ops, 1);
                ts.tlm = Mat::translate(op_number(ops, 0), op_number(ops, 1)).then(&ts.tlm);
                ts.tm = ts.tlm;
            }
            "Tm" => {
                ts.tlm = Mat {
                    a: op_number(ops, 0),
                    b: op_number(ops, 1),
                    c: op_number(ops, 2),
                    d: op_number(ops, 3),
                    e: op_number(ops, 4),
                    f: op_number(ops, 5),
                };
                ts.tm = ts.tlm;
            }
            "T*" => {
                ts.tlm = Mat::translate(0.0, -ts.leading).then(&ts.tlm);
                ts.tm = ts.tlm;
            }
            "TL" => ts.leading = op_number(ops, 0),
            "Tc" => ts.char_spacing = op_number(ops, 0),
            "Tw" => ts.word_spacing = op_number(ops, 0),
            "Tz" => ts.h_scale = op_number(ops, 0) / 100.0,
            "Ts" => ts.rise = op_number(ops, 0),
            "Tj" | "TJ" | "'" | "\"" => {
                if op.operator == "'" {
                    ts.tlm = Mat::translate(0.0, -ts.leading).then(&ts.tlm);
                    ts.tm = ts.tlm;
                } else if op.operator == "\"" {
                    ts.word_spacing = op_number(ops, 0);
                    ts.char_spacing = op_number(ops, 1);
                    ts.tlm = Mat::translate(0.0, -ts.leading).then(&ts.tlm);
                    ts.tm = ts.tlm;
                }

                let font_res = match &ts.font {
                    Some(f) => f.clone(),
                    None => continue,
                };
                let fm = match metrics.get(&font_res) {
                    Some(m) => m,
                    None => continue,
                };
                let Some(encoder) = encoders.get(&font_res) else { continue };

                let items: Vec<&Object> = match op.operator.as_str() {
                    "TJ" => match ops.first().and_then(|o| o.as_array().ok()) {
                        Some(arr) => arr.iter().collect(),
                        None => continue,
                    },
                    "\"" => ops.get(2).into_iter().collect(),
                    _ => ops.first().into_iter().collect(),
                };

                let mut text = String::new();
                let mut advance = 0.0f32;

                for item in items {
                    match item {
                        Object::String(bytes, _) => {
                            text.push_str(&encoder.decode(bytes));
                            for code in encoder.codes(bytes) {
                                let w = fm.width(code) / 1000.0 * ts.size + ts.char_spacing;
                                let w = if !fm.two_byte && code == 32 {
                                    w + ts.word_spacing
                                } else {
                                    w
                                };
                                advance += w * ts.h_scale;
                            }
                        }
                        other => {
                            if let Some(kern) = number(other) {
                                advance -= kern / 1000.0 * ts.size * ts.h_scale;
                            }
                        }
                    }
                }

                let m = ts.tm.then(&ctm);
                let corners = [
                    m.apply(0.0, fm.descent * ts.size + ts.rise),
                    m.apply(advance, fm.descent * ts.size + ts.rise),
                    m.apply(0.0, fm.ascent * ts.size + ts.rise),
                    m.apply(advance, fm.ascent * ts.size + ts.rise),
                ];
                let x0 = corners.iter().map(|p| p.0).fold(f32::MAX, f32::min);
                let x1 = corners.iter().map(|p| p.0).fold(f32::MIN, f32::max);
                let y0 = corners.iter().map(|p| p.1).fold(f32::MAX, f32::min);
                let y1 = corners.iter().map(|p| p.1).fold(f32::MIN, f32::max);

                let scale = (m.a * m.d - m.b * m.c).abs().sqrt();
                let visual_size = ts.size * if scale > 0.0 { scale } else { 1.0 };

                if !text.trim().is_empty() {
                    runs.push(TextRun {
                        op_index,
                        text,
                        font_res: font_res.clone(),
                        base_font: fm.base_font.clone(),
                        font_size: visual_size,
                        rect: [x0, y0, x1, y1],
                        two_byte: fm.two_byte,
                        editable: encoder.is_usable(),
                        map_source: encoder.source.label(),
                    });
                }

                ts.tm = Mat::translate(advance, 0.0).then(&ts.tm);
            }
            _ => {}
        }
    }

    Ok(runs)
}

/// Maps a point from PDF user space to rendered-image pixels, honouring /Rotate.
pub fn pdf_point_to_image(g: &crate::pdf::document::PageGeometry, dpi: u32, x: f32, y: f32) -> (f32, f32) {
    let s = dpi as f32 / 72.0;
    let u = x - g.x0;
    let v = (g.y0 + g.height) - y;
    let (u, v) = match g.rotate {
        90 => (g.height - v, u),
        180 => (g.width - u, g.height - v),
        270 => (v, g.width - u),
        _ => (u, v),
    };
    (u * s, v * s)
}

/// x0, y0, x1, y1 in image pixels.
pub fn pdf_rect_to_image(g: &crate::pdf::document::PageGeometry, dpi: u32, rect: [f32; 4]) -> [f32; 4] {
    let a = pdf_point_to_image(g, dpi, rect[0], rect[1]);
    let b = pdf_point_to_image(g, dpi, rect[2], rect[3]);
    [a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1)]
}

/// Inverse of [`pdf_point_to_image`]: rendered-image pixels back to user space.
pub fn image_point_to_pdf(g: &crate::pdf::document::PageGeometry, dpi: u32, px: f32, py: f32) -> (f32, f32) {
    let s = dpi as f32 / 72.0;
    let (u, v) = (px / s, py / s);
    let (u0, v0) = match g.rotate {
        90 => (v, g.height - u),
        180 => (g.width - u, g.height - v),
        270 => (g.width - v, u),
        _ => (u, v),
    };
    (g.x0 + u0, g.y0 + g.height - v0)
}

/// x0, y0, x1, y1 in user space, from an image-pixel rectangle.
pub fn image_rect_to_pdf(g: &crate::pdf::document::PageGeometry, dpi: u32, rect: [f32; 4]) -> [f32; 4] {
    let a = image_point_to_pdf(g, dpi, rect[0], rect[1]);
    let b = image_point_to_pdf(g, dpi, rect[2], rect[3]);
    [a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1)]
}
