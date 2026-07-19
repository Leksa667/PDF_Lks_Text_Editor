// =============================================================================
// PDF Lks Text Editor - Éditeur PDF avec OCR et édition de texte
// Créé par Leksa667 (https://github.com/Leksa667)
//
// Ce logiciel est en libre service. Vous pouvez l'utiliser, le modifier
// et le distribuer librement, à condition de créditer l'auteur original.
// Aucune garantie n'est fournie.
// =============================================================================

use crate::pdf::ttf::TtfFont;
use anyhow::{anyhow, Result};
use std::collections::{BTreeSet, HashMap};

/// Tables a PDF needs from an embedded TrueType font. `cmap` is deliberately
/// absent: an Identity-H composite font addresses glyphs by id, so the character
/// map is dead weight — /ToUnicode carries the meaning instead.
const KEPT: [&[u8; 4]; 6] = [b"glyf", b"head", b"hhea", b"hmtx", b"loca", b"maxp"];

fn be16(d: &[u8], off: usize) -> Option<u16> {
    Some(((*d.get(off)? as u16) << 8) | *d.get(off + 1)? as u16)
}

fn be32(d: &[u8], off: usize) -> Option<u32> {
    Some(((be16(d, off)? as u32) << 16) | be16(d, off + 2)? as u32)
}

fn table(data: &[u8], tag: &[u8; 4]) -> Option<(usize, usize)> {
    let count = be16(data, 4)? as usize;
    for i in 0..count {
        let rec = 12 + i * 16;
        if data.get(rec..rec + 4)? == tag.as_slice() {
            return Some((be32(data, rec + 8)? as usize, be32(data, rec + 12)? as usize));
        }
    }
    None
}

fn glyph_offsets(data: &[u8], num_glyphs: u16, long_loca: bool) -> Result<Vec<usize>> {
    let (loca, _) = table(data, b"loca").ok_or_else(|| anyhow!("Table loca absente"))?;
    let mut out = Vec::with_capacity(num_glyphs as usize + 1);
    for i in 0..=num_glyphs as usize {
        let offset = if long_loca {
            be32(data, loca + i * 4).ok_or_else(|| anyhow!("loca tronquée"))? as usize
        } else {
            be16(data, loca + i * 2).ok_or_else(|| anyhow!("loca tronquée"))? as usize * 2
        };
        out.push(offset);
    }
    Ok(out)
}

/// Offsets, inside a composite glyph, of every component's glyph-id field.
fn component_fields(glyph: &[u8]) -> Vec<usize> {
    let mut fields = Vec::new();
    if be16(glyph, 0).map(|v| v as i16).unwrap_or(0) >= 0 {
        return fields;
    }

    let mut off = 10;
    loop {
        let Some(flags) = be16(glyph, off) else { return fields };
        if glyph.len() < off + 4 {
            return fields;
        }
        fields.push(off + 2);

        off += if flags & 0x0001 != 0 { 8 } else { 6 };
        if flags & 0x0008 != 0 {
            off += 2;
        } else if flags & 0x0040 != 0 {
            off += 4;
        } else if flags & 0x0080 != 0 {
            off += 8;
        }
        if flags & 0x0020 == 0 {
            return fields;
        }
    }
}

/// A composite glyph draws other glyphs: they must travel with it.
fn add_components(
    glyf: &[u8],
    offsets: &[usize],
    gid: u16,
    wanted: &mut BTreeSet<u16>,
    depth: usize,
) {
    if depth > 8 {
        return;
    }
    let (Some(start), Some(end)) = (
        offsets.get(gid as usize).copied(),
        offsets.get(gid as usize + 1).copied(),
    ) else {
        return;
    };
    if end <= start || end > glyf.len() {
        return;
    }
    let glyph = &glyf[start..end];

    for field in component_fields(glyph) {
        let Some(component) = be16(glyph, field) else { continue };
        if wanted.insert(component) {
            add_components(glyf, offsets, component, wanted, depth + 1);
        }
    }
}

fn checksum(data: &[u8]) -> u32 {
    let mut sum = 0u32;
    for chunk in data.chunks(4) {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum = sum.wrapping_add(u32::from_be_bytes(word));
    }
    sum
}

fn pad4(bytes: &mut Vec<u8>) {
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
}

/// A subsetted font program plus the glyph-id renumbering it applied.
pub struct Subset {
    pub program: Vec<u8>,
    /// original glyph id -> glyph id inside the subset
    pub gid_map: HashMap<u16, u16>,
}

/// Rebuilds `font` with only the glyphs needed for `chars` (plus glyph 0 and any
/// components they reference), renumbering glyph ids compactly so `loca` and
/// `hmtx` shrink with the glyph count.
pub fn subset(font: &TtfFont, chars: &BTreeSet<char>) -> Result<Subset> {
    let data = &font.data;
    let num_glyphs = font.num_glyphs;

    let (head, head_len) = table(data, b"head").ok_or_else(|| anyhow!("Table head absente"))?;
    let long_loca = be16(data, head + 50).unwrap_or(0) == 1;
    let (glyf_off, glyf_len) = table(data, b"glyf").ok_or_else(|| anyhow!("Table glyf absente"))?;
    let glyf = data
        .get(glyf_off..glyf_off + glyf_len)
        .ok_or_else(|| anyhow!("Table glyf tronquée"))?;

    let offsets = glyph_offsets(data, num_glyphs, long_loca)?;

    let mut wanted: BTreeSet<u16> = BTreeSet::new();
    wanted.insert(0);
    for c in chars {
        if let Some(gid) = font.gid(*c) {
            if gid < num_glyphs {
                wanted.insert(gid);
            }
        }
    }
    for gid in wanted.clone() {
        add_components(glyf, &offsets, gid, &mut wanted, 0);
    }

    let gid_map: HashMap<u16, u16> = wanted
        .iter()
        .enumerate()
        .map(|(new, old)| (*old, new as u16))
        .collect();

    let mut new_glyf: Vec<u8> = Vec::new();
    let mut new_loca: Vec<u8> = Vec::with_capacity((wanted.len() + 1) * 4);
    let mut new_hmtx: Vec<u8> = Vec::with_capacity(wanted.len() * 4);

    for old_gid in &wanted {
        new_loca.extend_from_slice(&(new_glyf.len() as u32).to_be_bytes());

        let (start, end) = (offsets[*old_gid as usize], offsets[*old_gid as usize + 1]);
        if end > start && end <= glyf.len() {
            let mut glyph = glyf[start..end].to_vec();
            // A composite glyph names its parts by id: renumber them too.
            for field in component_fields(&glyph) {
                if let Some(component) = be16(&glyph, field) {
                    let renumbered = gid_map.get(&component).copied().unwrap_or(0);
                    glyph[field..field + 2].copy_from_slice(&renumbered.to_be_bytes());
                }
            }
            new_glyf.extend_from_slice(&glyph);
            pad4(&mut new_glyf);
        }

        let advance = font.advance_raw(*old_gid);
        new_hmtx.extend_from_slice(&advance.to_be_bytes());
        new_hmtx.extend_from_slice(&font.left_bearing_raw(*old_gid).to_be_bytes());
    }
    new_loca.extend_from_slice(&(new_glyf.len() as u32).to_be_bytes());

    let glyph_count = wanted.len() as u16;

    let mut tables: Vec<(&[u8; 4], Vec<u8>)> = Vec::new();
    for tag in KEPT {
        let bytes = match tag {
            b"glyf" => new_glyf.clone(),
            b"loca" => new_loca.clone(),
            b"hmtx" => new_hmtx.clone(),
            b"head" => {
                let mut head_bytes = data
                    .get(head..head + head_len)
                    .ok_or_else(|| anyhow!("Table head tronquée"))?
                    .to_vec();
                head_bytes[8..12].copy_from_slice(&[0, 0, 0, 0]); // checkSumAdjustment
                if head_bytes.len() > 51 {
                    head_bytes[50..52].copy_from_slice(&1u16.to_be_bytes()); // long loca
                }
                head_bytes
            }
            b"hhea" => {
                let (hhea, len) = table(data, b"hhea").ok_or_else(|| anyhow!("Table hhea absente"))?;
                let mut hhea_bytes = data
                    .get(hhea..hhea + len)
                    .ok_or_else(|| anyhow!("Table hhea tronquée"))?
                    .to_vec();
                // Every kept glyph now carries its own long metric.
                let n = hhea_bytes.len();
                hhea_bytes[n - 2..].copy_from_slice(&glyph_count.to_be_bytes());
                hhea_bytes
            }
            b"maxp" => {
                let (maxp, len) = table(data, b"maxp").ok_or_else(|| anyhow!("Table maxp absente"))?;
                let mut maxp_bytes = data
                    .get(maxp..maxp + len)
                    .ok_or_else(|| anyhow!("Table maxp tronquée"))?
                    .to_vec();
                maxp_bytes[4..6].copy_from_slice(&glyph_count.to_be_bytes());
                maxp_bytes
            }
            _ => continue,
        };
        tables.push((tag, bytes));
    }

    tables.sort_by(|a, b| a.0.cmp(b.0));

    let count = tables.len() as u16;
    let entry_selector = (count as f32).log2().floor() as u16;
    let search_range = 16 * 2u16.pow(entry_selector as u32);

    let mut out = Vec::new();
    out.extend_from_slice(&[0x00, 0x01, 0x00, 0x00]);
    out.extend_from_slice(&count.to_be_bytes());
    out.extend_from_slice(&search_range.to_be_bytes());
    out.extend_from_slice(&entry_selector.to_be_bytes());
    out.extend_from_slice(&(count * 16 - search_range).to_be_bytes());

    let mut offset = 12 + tables.len() * 16;
    let mut records = Vec::new();
    for (tag, bytes) in &tables {
        records.push((*tag, checksum(bytes), offset as u32, bytes.len() as u32));
        offset += bytes.len() + (4 - bytes.len() % 4) % 4;
    }

    for (tag, sum, off, len) in &records {
        out.extend_from_slice(tag.as_slice());
        out.extend_from_slice(&sum.to_be_bytes());
        out.extend_from_slice(&off.to_be_bytes());
        out.extend_from_slice(&len.to_be_bytes());
    }

    for (_, bytes) in &tables {
        out.extend_from_slice(bytes);
        pad4(&mut out);
    }

    Ok(Subset { program: out, gid_map })
}

/// Characters kept in every subset, so a later edit on the same page usually
/// needs no second font: printable ASCII, the Latin-1 range, and the typographic
/// marks French text is full of.
pub fn base_charset() -> BTreeSet<char> {
    let mut set: BTreeSet<char> = (0x20u32..0x7F).filter_map(char::from_u32).collect();
    set.extend((0xA0u32..0x100).filter_map(char::from_u32));
    set.extend([
        '\u{0152}', '\u{0153}', '\u{2013}', '\u{2014}', '\u{2018}', '\u{2019}', '\u{201C}',
        '\u{201D}', '\u{2022}', '\u{2026}', '\u{20AC}', '\u{2030}',
    ]);
    set
}
