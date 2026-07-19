// =============================================================================
// PDF Lks Text Editor - Éditeur PDF avec OCR et édition de texte
// Créé par Leksa667 (https://github.com/Leksa667)
//
// Ce logiciel est en libre service. Vous pouvez l'utiliser, le modifier
// et le distribuer librement, à condition de créditer l'auteur original.
// Aucune garantie n'est fournie.
// =============================================================================

use crate::pdf::document::{descendant_or_self, name_of};
use lopdf::{Dictionary, Document, Encoding};
use std::collections::HashMap;

/// Where the character map used for editing came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MapSource {
    ToUnicode,
    Encoding,
    EmbeddedCmap,
    None,
}

impl MapSource {
    pub fn label(&self) -> &'static str {
        match self {
            MapSource::ToUnicode => "ToUnicode",
            MapSource::Encoding => "Encoding",
            MapSource::EmbeddedCmap => "cmap intégré",
            MapSource::None => "aucune",
        }
    }
}

/// Bidirectional character map for one PDF font: decodes the codes found in
/// content streams and re-encodes arbitrary text back into that same font.
pub struct FontEncoder {
    pub two_byte: bool,
    pub source: MapSource,
    decode: HashMap<u32, String>,
    encode: HashMap<String, u32>,
    max_seq: usize,
}

impl FontEncoder {
    pub fn build(doc: &Document, font: &Dictionary) -> Self {
        let two_byte = name_of(font, b"Subtype").as_deref() == Some("Type0");
        let mut decode: HashMap<u32, String> = HashMap::new();
        let mut source = MapSource::None;

        // Lowest priority: the /Encoding table (glyph names, WinAnsi, MacRoman…).
        // Only simple fonts carry one; for Type0, /Encoding is a CMap name and
        // lopdf silently falls back to a one-byte table, which would be garbage.
        if !two_byte {
            if let Ok(enc) = font.get_font_encoding(doc) {
                let probed = probe_encoding(&enc);
                if !probed.is_empty() {
                    source = MapSource::Encoding;
                    decode = probed;
                }
            }
        }

        // Embedded TrueType/OpenType cmap: the only usable map for CID fonts
        // that ship without /ToUnicode.
        let cmap = embedded_cmap(doc, font, two_byte);
        if !cmap.is_empty() {
            for (code, ch) in &cmap {
                decode.entry(*code).or_insert_with(|| ch.to_string());
            }
            if source == MapSource::None {
                source = MapSource::EmbeddedCmap;
            }
        }

        // Highest priority: /ToUnicode is authoritative for what the glyphs mean.
        if let Some(map) = to_unicode_map(doc, font) {
            if !map.is_empty() {
                source = MapSource::ToUnicode;
                decode.extend(map);
            }
        }

        let mut encode: HashMap<String, u32> = HashMap::new();
        let mut max_seq = 1;
        for (code, text) in &decode {
            if text.is_empty() {
                continue;
            }
            max_seq = max_seq.max(text.chars().count());
            encode
                .entry(text.clone())
                .and_modify(|existing| *existing = (*existing).min(*code))
                .or_insert(*code);
        }
        // Characters the font can draw but /ToUnicode never mentions.
        for (code, ch) in cmap {
            encode.entry(ch.to_string()).or_insert(code);
        }

        Self { two_byte, source, decode, encode, max_seq }
    }

    pub fn is_usable(&self) -> bool {
        !self.encode.is_empty()
    }

    pub fn coverage(&self) -> usize {
        self.encode.len()
    }

    pub fn codes(&self, bytes: &[u8]) -> Vec<u32> {
        if self.two_byte {
            bytes
                .chunks(2)
                .map(|c| if c.len() == 2 { ((c[0] as u32) << 8) | c[1] as u32 } else { c[0] as u32 })
                .collect()
        } else {
            bytes.iter().map(|b| *b as u32).collect()
        }
    }

    pub fn decode(&self, bytes: &[u8]) -> String {
        let mut out = String::new();
        for code in self.codes(bytes) {
            if let Some(s) = self.decode.get(&code) {
                out.push_str(s);
            }
        }
        out
    }

    /// Encodes `text` into this font's codes, matching the longest character
    /// sequences first (so ligature glyphs such as "ﬁ" are used when present)
    /// and falling back to typographic equivalents when a character is absent.
    /// Returns the characters that the font simply cannot draw.
    pub fn encode(&self, text: &str) -> Result<Vec<u8>, String> {
        let chars: Vec<char> = text.chars().collect();
        let mut out = Vec::new();
        let mut missing = String::new();
        let mut i = 0;

        while i < chars.len() {
            let mut matched = 0;
            let limit = self.max_seq.min(chars.len() - i);
            for len in (1..=limit).rev() {
                let seq: String = chars[i..i + len].iter().collect();
                if let Some(code) = self.encode.get(&seq) {
                    push_code(&mut out, *code, self.two_byte);
                    matched = len;
                    break;
                }
            }
            if matched > 0 {
                i += matched;
                continue;
            }

            if let Some(replacement) = self.substitute(chars[i]) {
                for code in replacement {
                    push_code(&mut out, code, self.two_byte);
                }
                i += 1;
                continue;
            }

            if !missing.contains(chars[i]) {
                missing.push(chars[i]);
            }
            i += 1;
        }

        if !missing.is_empty() {
            return Err(missing);
        }
        Ok(out)
    }

    /// Typographic equivalents: apostrophes, quotes, dashes, ligatures, spaces.
    fn substitute(&self, c: char) -> Option<Vec<u32>> {
        let alternatives: &[&str] = match c {
            '\u{2019}' | '\u{2018}' | '\u{02BC}' => &["'"],
            '\u{201C}' | '\u{201D}' | '\u{201E}' => &["\""],
            '\u{2013}' | '\u{2014}' | '\u{2212}' | '\u{2011}' => &["-"],
            '\u{00A0}' | '\u{202F}' | '\u{2009}' | '\u{2007}' => &[" "],
            '\u{2026}' => &["..."],
            '\u{FB01}' => &["fi"],
            '\u{FB02}' => &["fl"],
            '\u{FB00}' => &["ff"],
            '\u{2022}' => &["\u{00B7}", "-"],
            '\u{20AC}' => &["EUR"],
            '\u{0153}' => &["oe"],
            '\u{0152}' => &["OE"],
            _ => return None,
        };

        for alt in alternatives {
            let mut codes = Vec::new();
            let mut ok = true;
            for ch in alt.chars() {
                match self.encode.get(&ch.to_string()) {
                    Some(code) => codes.push(*code),
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok && !codes.is_empty() {
                return Some(codes);
            }
        }
        None
    }
}

fn push_code(out: &mut Vec<u8>, code: u32, two_byte: bool) {
    if two_byte {
        out.push((code >> 8) as u8);
        out.push(code as u8);
    } else {
        out.push(code as u8);
    }
}

fn probe_encoding(enc: &Encoding) -> HashMap<u32, String> {
    let mut map = HashMap::new();
    for code in 0..=0xFFu32 {
        let Ok(text) = enc.bytes_to_string(&[code as u8]) else { continue };
        if text.is_empty() || text.contains('\u{FFFD}') {
            continue;
        }
        map.insert(code, text);
    }
    map
}

fn to_unicode_map(doc: &Document, font: &Dictionary) -> Option<HashMap<u32, String>> {
    let stream = font.get_deref(b"ToUnicode", doc).ok()?.as_stream().ok()?;
    let data = stream.decompressed_content().ok()?;
    Some(parse_to_unicode(&data))
}

#[derive(Debug)]
enum Token {
    Hex(Vec<u8>),
    Keyword(String),
    ArrayStart,
    ArrayEnd,
}

fn tokenize_cmap(data: &[u8]) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < data.len() {
        match data[i] {
            b'<' => {
                let mut hex = Vec::new();
                i += 1;
                while i < data.len() && data[i] != b'>' {
                    let c = data[i];
                    if c.is_ascii_hexdigit() {
                        hex.push(c);
                    }
                    i += 1;
                }
                i += 1;
                let bytes = hex
                    .chunks(2)
                    .filter(|c| c.len() == 2)
                    .filter_map(|c| u8::from_str_radix(std::str::from_utf8(c).ok()?, 16).ok())
                    .collect();
                tokens.push(Token::Hex(bytes));
            }
            b'[' => {
                tokens.push(Token::ArrayStart);
                i += 1;
            }
            b']' => {
                tokens.push(Token::ArrayEnd);
                i += 1;
            }
            c if c.is_ascii_alphabetic() => {
                let start = i;
                while i < data.len() && (data[i].is_ascii_alphanumeric() || data[i] == b'.') {
                    i += 1;
                }
                tokens.push(Token::Keyword(
                    String::from_utf8_lossy(&data[start..i]).to_string(),
                ));
            }
            _ => i += 1,
        }
    }
    tokens
}

fn code_of(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0u32, |acc, b| (acc << 8) | *b as u32)
}

fn utf16be_to_string(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| ((c[0] as u16) << 8) | c[1] as u16)
        .collect();
    String::from_utf16_lossy(&units)
}

/// Parses a /ToUnicode CMap: `beginbfchar` pairs and `beginbfrange` ranges,
/// including the array form `<lo> <hi> [<d1> <d2> …]`.
pub fn parse_to_unicode(data: &[u8]) -> HashMap<u32, String> {
    let tokens = tokenize_cmap(data);
    let mut map = HashMap::new();
    let mut i = 0;

    while i < tokens.len() {
        match &tokens[i] {
            Token::Keyword(k) if k == "beginbfchar" => {
                i += 1;
                while i + 1 < tokens.len() {
                    let (Token::Hex(src), Token::Hex(dst)) = (&tokens[i], &tokens[i + 1]) else {
                        break;
                    };
                    let text = utf16be_to_string(dst);
                    if !text.is_empty() && !text.contains('\u{0}') {
                        map.insert(code_of(src), text);
                    }
                    i += 2;
                }
            }
            Token::Keyword(k) if k == "beginbfrange" => {
                i += 1;
                while i + 2 < tokens.len() {
                    let (Token::Hex(lo), Token::Hex(hi)) = (&tokens[i], &tokens[i + 1]) else {
                        break;
                    };
                    let (lo_code, hi_code) = (code_of(lo), code_of(hi));
                    i += 2;

                    match &tokens[i] {
                        Token::Hex(dst) => {
                            let base = utf16be_to_string(dst);
                            let mut chars: Vec<char> = base.chars().collect();
                            for code in lo_code..=hi_code.min(lo_code + 65535) {
                                if chars.is_empty() {
                                    break;
                                }
                                let text: String = chars.iter().collect();
                                map.insert(code, text);
                                // Successive codes map to successive characters.
                                let last = chars.len() - 1;
                                chars[last] = char::from_u32(chars[last] as u32 + 1).unwrap_or(chars[last]);
                            }
                            i += 1;
                        }
                        Token::ArrayStart => {
                            i += 1;
                            let mut code = lo_code;
                            while i < tokens.len() {
                                match &tokens[i] {
                                    Token::Hex(dst) => {
                                        let text = utf16be_to_string(dst);
                                        if !text.is_empty() {
                                            map.insert(code, text);
                                        }
                                        code += 1;
                                        i += 1;
                                    }
                                    _ => break,
                                }
                            }
                            if matches!(tokens.get(i), Some(Token::ArrayEnd)) {
                                i += 1;
                            }
                        }
                        _ => break,
                    }
                }
            }
            _ => i += 1,
        }
    }

    map
}

fn embedded_cmap(doc: &Document, font: &Dictionary, two_byte: bool) -> HashMap<u32, char> {
    let mut out = HashMap::new();
    if !two_byte {
        return out;
    }
    let Some(descendant) = descendant_or_self(doc, font) else { return out };
    let Ok(descriptor) = doc.get_dict_in_dict(descendant, b"FontDescriptor") else { return out };

    let mut data = None;
    for key in [b"FontFile2".as_slice(), b"FontFile3".as_slice()] {
        if let Ok(obj) = descriptor.get_deref(key, doc) {
            if let Ok(stream) = obj.as_stream() {
                if let Ok(bytes) = stream.decompressed_content() {
                    data = Some(bytes);
                    break;
                }
            }
        }
    }
    let Some(data) = data else { return out };
    let unicode_to_gid = parse_truetype_cmap(&data);
    if unicode_to_gid.is_empty() {
        return out;
    }

    let gid_to_cid = cid_to_gid_inverse(doc, descendant);
    for (ch, gid) in unicode_to_gid {
        let cid = match &gid_to_cid {
            Some(map) => match map.get(&gid) {
                Some(cid) => *cid,
                None => continue,
            },
            None => gid as u32,
        };
        out.entry(cid).or_insert(ch);
    }
    out
}

/// /CIDToGIDMap streams map CID -> GID; editing needs the reverse.
fn cid_to_gid_inverse(doc: &Document, descendant: &Dictionary) -> Option<HashMap<u16, u32>> {
    let obj = descendant.get_deref(b"CIDToGIDMap", doc).ok()?;
    let stream = obj.as_stream().ok()?;
    let data = stream.decompressed_content().ok()?;
    let mut map = HashMap::new();
    for (cid, pair) in data.chunks_exact(2).enumerate() {
        let gid = ((pair[0] as u16) << 8) | pair[1] as u16;
        if gid != 0 {
            map.entry(gid).or_insert(cid as u32);
        }
    }
    Some(map)
}

fn be16(d: &[u8], off: usize) -> Option<u16> {
    Some(((*d.get(off)? as u16) << 8) | *d.get(off + 1)? as u16)
}

fn be32(d: &[u8], off: usize) -> Option<u32> {
    Some(((be16(d, off)? as u32) << 16) | be16(d, off + 2)? as u32)
}

/// Minimal TrueType/OpenType `cmap` reader (formats 0, 4, 6 and 12).
pub fn parse_truetype_cmap(data: &[u8]) -> HashMap<char, u16> {
    let mut out = HashMap::new();
    let num_tables = match be16(data, 4) {
        Some(n) => n as usize,
        None => return out,
    };

    let mut cmap_off = None;
    for i in 0..num_tables {
        let rec = 12 + i * 16;
        if data.get(rec..rec + 4) == Some(b"cmap") {
            cmap_off = be32(data, rec + 8).map(|v| v as usize);
            break;
        }
    }
    let Some(cmap) = cmap_off else { return out };

    let count = match be16(data, cmap + 2) {
        Some(c) => c as usize,
        None => return out,
    };

    let mut best: Option<(u32, usize)> = None;
    for i in 0..count {
        let rec = cmap + 4 + i * 8;
        let (Some(platform), Some(encoding), Some(offset)) =
            (be16(data, rec), be16(data, rec + 2), be32(data, rec + 4))
        else {
            continue;
        };
        // Prefer full Unicode, then BMP Unicode, then Mac/symbol tables.
        let score = match (platform, encoding) {
            (3, 10) | (0, 4) | (0, 6) => 4,
            (3, 1) | (0, 3) => 3,
            (0, _) => 2,
            (3, 0) => 1,
            _ => 0,
        };
        if score == 0 {
            continue;
        }
        if best.is_none_or(|(s, _)| score > s) {
            best = Some((score, cmap + offset as usize));
        }
    }
    let Some((score, sub)) = best else { return out };
    let symbol = score == 1;

    match be16(data, sub) {
        Some(0) => {
            for code in 0u32..256 {
                if let Some(gid) = data.get(sub + 6 + code as usize) {
                    insert_gid(&mut out, code, *gid as u16, symbol);
                }
            }
        }
        Some(4) => {
            let Some(seg_x2) = be16(data, sub + 6) else { return out };
            let segs = seg_x2 as usize / 2;
            let ends = sub + 14;
            let starts = ends + seg_x2 as usize + 2;
            let deltas = starts + seg_x2 as usize;
            let ranges = deltas + seg_x2 as usize;

            for s in 0..segs {
                let (Some(end), Some(start), Some(delta), Some(range_off)) = (
                    be16(data, ends + s * 2),
                    be16(data, starts + s * 2),
                    be16(data, deltas + s * 2),
                    be16(data, ranges + s * 2),
                ) else {
                    continue;
                };
                if start == 0xFFFF {
                    continue;
                }
                for code in start..=end {
                    let gid = if range_off == 0 {
                        code.wrapping_add(delta)
                    } else {
                        let idx = ranges + s * 2 + range_off as usize + (code - start) as usize * 2;
                        match be16(data, idx) {
                            Some(0) | None => continue,
                            Some(g) => g.wrapping_add(delta),
                        }
                    };
                    insert_gid(&mut out, code as u32, gid, symbol);
                    if code == 0xFFFF {
                        break;
                    }
                }
            }
        }
        Some(6) => {
            let (Some(first), Some(count)) = (be16(data, sub + 6), be16(data, sub + 8)) else {
                return out;
            };
            for i in 0..count as usize {
                if let Some(gid) = be16(data, sub + 10 + i * 2) {
                    insert_gid(&mut out, first as u32 + i as u32, gid, symbol);
                }
            }
        }
        Some(12) => {
            let Some(groups) = be32(data, sub + 12) else { return out };
            for g in 0..groups.min(100_000) as usize {
                let rec = sub + 16 + g * 12;
                let (Some(start), Some(end), Some(start_gid)) =
                    (be32(data, rec), be32(data, rec + 4), be32(data, rec + 8))
                else {
                    continue;
                };
                for code in start..=end.min(start + 65535) {
                    let gid = start_gid + (code - start);
                    insert_gid(&mut out, code, gid as u16, symbol);
                }
            }
        }
        _ => {}
    }

    out
}

fn insert_gid(out: &mut HashMap<char, u16>, code: u32, gid: u16, symbol: bool) {
    if gid == 0 {
        return;
    }
    // Symbol fonts map their glyphs into the private-use area at 0xF000.
    let code = if symbol && (0xF000..=0xF0FF).contains(&code) {
        code - 0xF000
    } else {
        code
    };
    if let Some(ch) = char::from_u32(code) {
        out.entry(ch).or_insert(gid);
    }
}
