// =============================================================================
// PDF Lks Text Editor - Éditeur PDF avec OCR et édition de texte
// Créé par Leksa667 (https://github.com/Leksa667)
//
// Ce logiciel est en libre service. Vous pouvez l'utiliser, le modifier
// et le distribuer librement, à condition de créditer l'auteur original.
// Aucune garantie n'est fournie.
// =============================================================================

use crate::pdf::document::number;
use crate::pdf::font::FontEncoder;
use anyhow::{anyhow, Result};
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, ObjectId, StringFormat};

fn encode_with_font(doc: &Document, page_id: ObjectId, font_res: &str, text: &str) -> Result<Vec<u8>> {
    let fonts = doc
        .get_page_fonts(page_id)
        .map_err(|e| anyhow!("Polices illisibles : {e}"))?;
    let font = fonts
        .get(font_res.as_bytes())
        .ok_or_else(|| anyhow!("Police /{font_res} introuvable"))?;

    let encoder = FontEncoder::build(doc, font);
    if !encoder.is_usable() {
        return Err(anyhow!(
            "Police sans table de caractères exploitable (ni ToUnicode, ni Encoding, ni cmap intégré)"
        ));
    }

    encoder.encode(text).map_err(|missing| {
        anyhow!(
            "Caractères absents de la police ({}) : {}",
            encoder.source.label(),
            missing.chars().take(12).collect::<String>()
        )
    })
}

/// Rewrites the string operand of one show-text operation, preserving font,
/// size and position.
pub fn set_run_text(
    doc: &mut Document,
    page_id: ObjectId,
    op_index: usize,
    font_res: &str,
    two_byte: bool,
    new_text: &str,
) -> Result<()> {
    let bytes = encode_with_font(doc, page_id, font_res, new_text)?;
    let format = if two_byte {
        StringFormat::Hexadecimal
    } else {
        StringFormat::Literal
    };
    let string = Object::String(bytes, format);

    let data = doc.get_page_content(page_id);
    let mut content = Content::decode(&data).map_err(|e| anyhow!("Flux illisible : {e}"))?;

    let op = content
        .operations
        .get_mut(op_index)
        .ok_or_else(|| anyhow!("Opération {op_index} introuvable"))?;

    match op.operator.as_str() {
        "Tj" | "'" => op.operands = vec![string],
        "TJ" => op.operands = vec![Object::Array(vec![string])],
        "\"" => {
            while op.operands.len() < 3 {
                op.operands.push(Object::Integer(0));
            }
            op.operands[2] = string;
        }
        other => return Err(anyhow!("Opérateur {other} non éditable")),
    }

    let encoded = content
        .encode()
        .map_err(|e| anyhow!("Ré-encodage du flux impossible : {e}"))?;
    doc.change_page_content(page_id, encoded)
        .map_err(|e| anyhow!("Écriture du flux impossible : {e}"))?;
    Ok(())
}

/// Translates a text run by `(dx, dy)` in PDF user space by adjusting the
/// last `Tm` or `Td`/`TD` that precedes the show-text operation. If none is
/// found inside the text block, a `Tm` is inserted.
pub fn move_run_by(
    doc: &mut Document,
    page_id: ObjectId,
    op_index: usize,
    dx: f32,
    dy: f32,
) -> Result<()> {
    if dx == 0.0 && dy == 0.0 {
        return Ok(());
    }
    let data = doc.get_page_content(page_id);
    let mut content = Content::decode(&data).map_err(|e| anyhow!("Flux illisible : {e}"))?;

    let search_end = op_index.min(content.operations.len());
    let mut found = false;
    for i in (0..search_end).rev() {
        let operator = content.operations[i].operator.clone();
        match operator.as_str() {
            "Tm" => {
                if content.operations[i].operands.len() >= 6 {
                    let e = number(&content.operations[i].operands[4]).unwrap_or(0.0);
                    let f = number(&content.operations[i].operands[5]).unwrap_or(0.0);
                    content.operations[i].operands[4] = Object::Real(e + dx);
                    content.operations[i].operands[5] = Object::Real(f + dy);
                    found = true;
                }
                break;
            }
            "Td" | "TD" => {
                if content.operations[i].operands.len() >= 2 {
                    let tx = number(&content.operations[i].operands[0]).unwrap_or(0.0);
                    let ty = number(&content.operations[i].operands[1]).unwrap_or(0.0);
                    content.operations[i].operands[0] = Object::Real(tx + dx);
                    content.operations[i].operands[1] = Object::Real(ty + dy);
                    found = true;
                }
                break;
            }
            "BT" => break,
            _ => {}
        }
        if found {
            break;
        }
    }

    if !found {
        let tm = Operation::new(
            "Tm",
            vec![
                Object::Real(1.0),
                Object::Real(0.0),
                Object::Real(0.0),
                Object::Real(1.0),
                Object::Real(dx),
                Object::Real(dy),
            ],
        );
        content.operations.insert(op_index, tm);
    }

    let encoded = content
        .encode()
        .map_err(|e| anyhow!("Ré-encodage du flux impossible : {e}"))?;
    doc.change_page_content(page_id, encoded)
        .map_err(|e| anyhow!("Écriture du flux impossible : {e}"))?;
    Ok(())
}
