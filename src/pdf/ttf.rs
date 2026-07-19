// =============================================================================
// PDF Lks Text Editor - Éditeur PDF avec OCR et édition de texte
// Créé par Leksa667 (https://github.com/Leksa667)
//
// Ce logiciel est en libre service. Vous pouvez l'utiliser, le modifier
// et le distribuer librement, à condition de créditer l'auteur original.
// Aucune garantie n'est fournie.
// =============================================================================

use crate::pdf::font::parse_truetype_cmap;
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::path::Path;

/// The subset of TrueType metadata a PDF needs in order to embed the font.
pub struct TtfFont {
    pub data: Vec<u8>,
    pub family: String,
    pub units_per_em: f32,
    pub ascent: f32,
    pub descent: f32,
    pub cap_height: f32,
    pub italic_angle: f32,
    pub bbox: [i64; 4],
    pub flags: i64,
    pub num_glyphs: u16,
    pub bold: bool,
    pub italic: bool,
    pub cmap: HashMap<char, u16>,
    advances: Vec<u16>,
    advances_raw: Vec<u16>,
    bearings_raw: Vec<i16>,
}

fn be16(d: &[u8], off: usize) -> Option<u16> {
    Some(((*d.get(off)? as u16) << 8) | *d.get(off + 1)? as u16)
}

fn be32(d: &[u8], off: usize) -> Option<u32> {
    Some(((be16(d, off)? as u32) << 16) | be16(d, off + 2)? as u32)
}

fn i16_at(d: &[u8], off: usize) -> i64 {
    be16(d, off).map(|v| v as i16 as i64).unwrap_or(0)
}

fn table(data: &[u8], tag: &[u8; 4]) -> Option<(usize, usize)> {
    let count = be16(data, 4)? as usize;
    for i in 0..count {
        let rec = 12 + i * 16;
        if data.get(rec..rec + 4)? == tag {
            let off = be32(data, rec + 8)? as usize;
            let len = be32(data, rec + 12)? as usize;
            return Some((off, len));
        }
    }
    None
}

/// Reads the family name from the `name` table (Windows Unicode records first).
fn family_name(data: &[u8]) -> Option<String> {
    let (off, _) = table(data, b"name")?;
    let count = be16(data, off + 2)? as usize;
    let storage = off + be16(data, off + 4)? as usize;
    let mut fallback = None;

    for i in 0..count {
        let rec = off + 6 + i * 12;
        let platform = be16(data, rec)?;
        let name_id = be16(data, rec + 6)?;
        if name_id != 1 {
            continue;
        }
        let len = be16(data, rec + 8)? as usize;
        let str_off = storage + be16(data, rec + 10)? as usize;
        let bytes = data.get(str_off..str_off + len)?;

        let text = if platform == 3 || platform == 0 {
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|c| ((c[0] as u16) << 8) | c[1] as u16)
                .collect();
            String::from_utf16_lossy(&units)
        } else {
            String::from_utf8_lossy(bytes).to_string()
        };
        if platform == 3 {
            return Some(text);
        }
        fallback.get_or_insert(text);
    }
    fallback
}

impl TtfFont {
    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read(path)
            .map_err(|e| anyhow!("Lecture de {} impossible : {e}", path.display()))?;
        Self::parse(data)
    }

    pub fn parse(data: Vec<u8>) -> Result<Self> {
        if table(&data, b"glyf").is_none() {
            return Err(anyhow!(
                "Police non TrueType (CFF/OpenType non supporté pour l'embarquement)"
            ));
        }
        let (head, _) = table(&data, b"head").ok_or_else(|| anyhow!("Table head absente"))?;
        let units_per_em = be16(&data, head + 18).unwrap_or(1000) as f32;
        let scale = 1000.0 / units_per_em.max(1.0);
        let bbox = [
            (i16_at(&data, head + 36) as f32 * scale) as i64,
            (i16_at(&data, head + 38) as f32 * scale) as i64,
            (i16_at(&data, head + 40) as f32 * scale) as i64,
            (i16_at(&data, head + 42) as f32 * scale) as i64,
        ];

        let (maxp, _) = table(&data, b"maxp").ok_or_else(|| anyhow!("Table maxp absente"))?;
        let num_glyphs = be16(&data, maxp + 4).unwrap_or(0);

        let (hhea, _) = table(&data, b"hhea").ok_or_else(|| anyhow!("Table hhea absente"))?;
        let num_h_metrics = be16(&data, hhea + 34).unwrap_or(0) as usize;
        let mut ascent = i16_at(&data, hhea + 4) as f32 * scale;
        let mut descent = i16_at(&data, hhea + 6) as f32 * scale;

        let mut cap_height = ascent * 0.7;
        let mut flags = 32;
        let mut bold = false;
        let mut italic = false;
        if let Some((os2, len)) = table(&data, b"OS/2") {
            // usWeightClass, then the fsSelection style bits.
            bold = be16(&data, os2 + 4).unwrap_or(400) >= 600;
            let selection = be16(&data, os2 + 62).unwrap_or(0);
            bold |= selection & 0x0020 != 0;
            italic = selection & 0x0001 != 0;
            let typo_ascent = i16_at(&data, os2 + 68) as f32 * scale;
            let typo_descent = i16_at(&data, os2 + 70) as f32 * scale;
            if typo_ascent > 0.0 {
                ascent = typo_ascent;
                descent = typo_descent;
            }
            if len >= 90 {
                let ch = i16_at(&data, os2 + 88) as f32 * scale;
                if ch > 0.0 {
                    cap_height = ch;
                }
            }
            if italic {
                flags |= 64;
            }
        }

        let italic_angle = table(&data, b"post")
            .and_then(|(post, _)| be32(&data, post + 4))
            .map(|fixed| fixed as i32 as f32 / 65536.0)
            .unwrap_or(0.0);

        let mut advances = Vec::with_capacity(num_glyphs as usize);
        let mut advances_raw = Vec::with_capacity(num_glyphs as usize);
        let mut bearings_raw = Vec::with_capacity(num_glyphs as usize);
        if let Some((hmtx, _)) = table(&data, b"hmtx") {
            let mut last = 0u16;
            for gid in 0..num_glyphs as usize {
                let (advance, bearing) = if gid < num_h_metrics {
                    last = be16(&data, hmtx + gid * 4).unwrap_or(last);
                    (last, i16_at(&data, hmtx + gid * 4 + 2) as i16)
                } else {
                    let extra = gid - num_h_metrics;
                    let base = hmtx + num_h_metrics * 4 + extra * 2;
                    (last, i16_at(&data, base) as i16)
                };
                advances.push((advance as f32 * scale) as u16);
                advances_raw.push(advance);
                bearings_raw.push(bearing);
            }
        }

        let cmap = parse_truetype_cmap(&data);
        if cmap.is_empty() {
            return Err(anyhow!("Police sans table cmap exploitable"));
        }

        let family = family_name(&data).unwrap_or_else(|| "Inconnue".to_string());

        Ok(Self {
            data,
            family,
            units_per_em,
            ascent,
            descent,
            cap_height,
            italic_angle,
            bbox,
            flags,
            num_glyphs,
            bold,
            italic,
            cmap,
            advances,
            advances_raw,
            bearings_raw,
        })
    }

    /// "Verdana Bold Italic", from the family plus the style bits.
    pub fn style_name(&self) -> String {
        match (self.bold, self.italic) {
            (true, true) => format!("{} Bold Italic", self.family),
            (true, false) => format!("{} Bold", self.family),
            (false, true) => format!("{} Italic", self.family),
            (false, false) => self.family.clone(),
        }
    }

    pub fn gid(&self, c: char) -> Option<u16> {
        self.cmap.get(&c).copied()
    }

    /// Advance width of a glyph, in 1/1000 em.
    pub fn advance(&self, gid: u16) -> f32 {
        self.advances.get(gid as usize).copied().unwrap_or(500) as f32
    }

    /// Advance width in the font's own units, as hmtx stores it.
    pub fn advance_raw(&self, gid: u16) -> u16 {
        self.advances_raw.get(gid as usize).copied().unwrap_or(0)
    }

    /// Left side bearing in the font's own units, as hmtx stores it.
    pub fn left_bearing_raw(&self, gid: u16) -> i16 {
        self.bearings_raw.get(gid as usize).copied().unwrap_or(0)
    }

    pub fn text_width(&self, text: &str, size: f32) -> f32 {
        text.chars()
            .filter_map(|c| self.gid(c))
            .map(|g| self.advance(g) / 1000.0 * size)
            .sum()
    }

    /// PDF-side name: no spaces allowed in a /BaseFont name.
    pub fn pdf_name(&self) -> String {
        self.family
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect::<String>()
    }

    pub fn missing_chars(&self, text: &str) -> String {
        let mut missing = String::new();
        for c in text.chars() {
            if self.gid(c).is_none() && !missing.contains(c) {
                missing.push(c);
            }
        }
        missing
    }
}
