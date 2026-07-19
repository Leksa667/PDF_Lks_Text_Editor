// =============================================================================
// PDF Lks Text Editor - Éditeur PDF avec OCR et édition de texte
// Créé par Leksa667 (https://github.com/Leksa667)
//
// Ce logiciel est en libre service. Vous pouvez l'utiliser, le modifier
// et le distribuer librement, à condition de créditer l'auteur original.
// Aucune garantie n'est fournie.
// =============================================================================

use anyhow::{anyhow, Context, Result};
use lopdf::{Dictionary, Document, Object, ObjectId};
use std::path::Path;

/// Above this many characters, a page is real text, not a scan with a stray layer.
const SCAN_TEXT_LIMIT: usize = 200;

#[derive(Clone, Debug)]
pub struct FontInfo {
    pub internal_name: String,
    pub base_font: String,
    pub font_type: String,
    pub embedded: bool,
    pub map_source: &'static str,
    pub coverage: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct PageGeometry {
    pub x0: f32,
    pub y0: f32,
    pub width: f32,
    pub height: f32,
    pub rotate: i64,
}

pub struct PdfDocument {
    pub doc: Document,
    pub page_ids: Vec<ObjectId>,
    undo_stack: Vec<(ObjectId, Vec<u8>)>,
    redo_stack: Vec<(ObjectId, Vec<u8>)>,
    pub dirty: bool,
}

impl PdfDocument {
    pub fn open(path: &Path) -> Result<Self> {
        let doc = Document::load(path)
            .with_context(|| format!("Impossible d'ouvrir {}", path.display()))?;
        let page_ids: Vec<ObjectId> = doc.get_pages().values().copied().collect();
        if page_ids.is_empty() {
            return Err(anyhow!("Le document ne contient aucune page"));
        }
        Ok(Self {
            doc,
            page_ids,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            dirty: false,
        })
    }

    pub fn page_count(&self) -> usize {
        self.page_ids.len()
    }

    pub fn page_id(&self, page: usize) -> Result<ObjectId> {
        self.page_ids
            .get(page)
            .copied()
            .ok_or_else(|| anyhow!("Page {} inexistante", page + 1))
    }

    pub fn geometry(&self, page: usize) -> Result<PageGeometry> {
        let page_id = self.page_id(page)?;
        let media = self
            .inherited(page_id, b"MediaBox")
            .and_then(|o| rect_from_object(&self.doc, &o))
            .unwrap_or([0.0, 0.0, 612.0, 792.0]);
        let rotate = self
            .inherited(page_id, b"Rotate")
            .and_then(|o| o.as_i64().ok())
            .unwrap_or(0)
            .rem_euclid(360);

        Ok(PageGeometry {
            x0: media[0].min(media[2]),
            y0: media[1].min(media[3]),
            width: (media[2] - media[0]).abs(),
            height: (media[3] - media[1]).abs(),
            rotate,
        })
    }

    fn inherited(&self, page_id: ObjectId, key: &[u8]) -> Option<Object> {
        let mut current = page_id;
        for _ in 0..32 {
            let dict = self.doc.get_dictionary(current).ok()?;
            if let Ok(obj) = dict.get(key) {
                if let Ok((_, resolved)) = self.doc.dereference(obj) {
                    return Some(resolved.clone());
                }
            }
            current = dict.get(b"Parent").ok()?.as_reference().ok()?;
        }
        None
    }

    pub fn fonts(&self, page: usize) -> Result<Vec<FontInfo>> {
        let page_id = self.page_id(page)?;
        let mut fonts = Vec::new();
        let page_fonts = match self.doc.get_page_fonts(page_id) {
            Ok(f) => f,
            Err(_) => return Ok(fonts),
        };

        for (key, dict) in page_fonts {
            let base_font = name_of(dict, b"BaseFont").unwrap_or_else(|| "Inconnue".to_string());
            let font_type = name_of(dict, b"Subtype").unwrap_or_else(|| "Inconnu".to_string());
            let embedded = descendant_or_self(&self.doc, dict)
                .and_then(|d| self.doc.get_dict_in_dict(d, b"FontDescriptor").ok())
                .map(|fd| fd.has(b"FontFile") || fd.has(b"FontFile2") || fd.has(b"FontFile3"))
                .unwrap_or(false);

            let encoder = crate::pdf::font::FontEncoder::build(&self.doc, dict);
            fonts.push(FontInfo {
                internal_name: String::from_utf8_lossy(&key).to_string(),
                base_font,
                font_type,
                embedded,
                map_source: encoder.source.label(),
                coverage: encoder.coverage(),
            });
        }
        fonts.sort_by(|a, b| a.internal_name.cmp(&b.internal_name));
        Ok(fonts)
    }

    /// A page that carries an image is a scan candidate: copiers often also
    /// leave a stray text layer on it, so the presence of a few text runs must
    /// not disqualify it.
    pub fn image_count(&self, page: usize) -> usize {
        let Ok(page_id) = self.page_id(page) else { return 0 };
        self.doc.get_page_images(page_id).map(|i| i.len()).unwrap_or(0)
    }

    /// A scan is a page made of an image carrying little or no real text.
    /// Judging on "no text at all" is wrong: copiers leave stray text layers,
    /// and once this editor has written a correction onto a scan the page holds
    /// text of its own — it must still be treated as a scan.
    pub fn is_scanned(&self, page: usize) -> bool {
        if self.image_count(page) == 0 {
            return false;
        }
        let Ok(page_id) = self.page_id(page) else { return false };
        let chars: usize = crate::pdf::text::extract_runs(&self.doc, page_id)
            .map(|runs| runs.iter().map(|r| r.text.chars().count()).sum())
            .unwrap_or(0);
        chars < SCAN_TEXT_LIMIT
    }

    /// Native resolution of the scan on this page, in dpi.
    ///
    /// Rendering a scan above its own resolution only upsamples blur, and font
    /// matching then compares mush. The largest image on the page tells us the
    /// resolution the paper was actually digitised at.
    pub fn scan_dpi(&self, page: usize) -> Option<u32> {
        let page_id = self.page_id(page).ok()?;
        let geometry = self.geometry(page).ok()?;
        let images = self.doc.get_page_images(page_id).ok()?;
        let biggest = images.iter().max_by_key(|i| i.width as u64 * i.height as u64)?;

        if geometry.width <= 0.0 || geometry.height <= 0.0 {
            return None;
        }
        // The image covers the page, in unrotated user space.
        let dpi_x = biggest.width as f32 / (geometry.width / 72.0);
        let dpi_y = biggest.height as f32 / (geometry.height / 72.0);
        Some(((dpi_x + dpi_y) / 2.0).round() as u32)
    }

    pub fn record_undo(&mut self, page_id: ObjectId, before: Vec<u8>) {
        self.undo_stack.push((page_id, before));
        self.redo_stack.clear();
        if self.undo_stack.len() > 100 {
            self.undo_stack.remove(0);
        }
        self.dirty = true;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn undo(&mut self) -> Option<ObjectId> {
        let (page_id, content) = self.undo_stack.pop()?;
        let current = self.doc.get_page_content(page_id);
        if self.doc.change_page_content(page_id, content).is_err() {
            return None;
        }
        self.redo_stack.push((page_id, current));
        self.dirty = true;
        Some(page_id)
    }

    pub fn redo(&mut self) -> Option<ObjectId> {
        let (page_id, content) = self.redo_stack.pop()?;
        let current = self.doc.get_page_content(page_id);
        if self.doc.change_page_content(page_id, content).is_err() {
            return None;
        }
        self.undo_stack.push((page_id, current));
        self.dirty = true;
        Some(page_id)
    }

    pub fn save(&mut self, path: &Path) -> Result<()> {
        self.doc
            .save(path)
            .with_context(|| format!("Impossible de sauvegarder {}", path.display()))?;
        self.dirty = false;
        Ok(())
    }

    pub fn write_snapshot(&mut self, path: &Path) -> Result<()> {
        self.doc
            .save(path)
            .with_context(|| format!("Impossible d'écrire {}", path.display()))?;
        Ok(())
    }
}

pub fn descendant_or_self<'a>(doc: &'a Document, font: &'a Dictionary) -> Option<&'a Dictionary> {
    if let Ok(obj) = font.get(b"DescendantFonts") {
        let (_, resolved) = doc.dereference(obj).ok()?;
        let arr = resolved.as_array().ok()?;
        let first = arr.first()?;
        let (_, desc) = doc.dereference(first).ok()?;
        return desc.as_dict().ok();
    }
    Some(font)
}

fn rect_from_object(doc: &Document, obj: &Object) -> Option<[f32; 4]> {
    let arr = obj.as_array().ok()?;
    if arr.len() < 4 {
        return None;
    }
    let mut out = [0.0f32; 4];
    for (i, item) in arr.iter().take(4).enumerate() {
        let (_, resolved) = doc.dereference(item).ok()?;
        out[i] = number(resolved)?;
    }
    Some(out)
}

pub fn number(obj: &Object) -> Option<f32> {
    match obj {
        Object::Integer(i) => Some(*i as f32),
        Object::Real(r) => Some(*r),
        _ => None,
    }
}

pub fn name_of(dict: &Dictionary, key: &[u8]) -> Option<String> {
    let obj = dict.get(key).ok()?;
    if let Ok(name) = obj.as_name() {
        return Some(String::from_utf8_lossy(name).to_string());
    }
    if let Ok(s) = obj.as_str() {
        return Some(String::from_utf8_lossy(s).to_string());
    }
    None
}
