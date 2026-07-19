// =============================================================================
// PDF Lks Text Editor - Éditeur PDF avec OCR et édition de texte
// Créé par Leksa667 (https://github.com/Leksa667)
//
// Ce logiciel est en libre service. Vous pouvez l'utiliser, le modifier
// et le distribuer librement, à condition de créditer l'auteur original.
// Aucune garantie n'est fournie.
// =============================================================================

use crate::ocr::fontmatch::FontMatch;
use crate::ocr::tesseract::OcrWord;
use crate::pdf::document::{FontInfo, PageGeometry, PdfDocument};
use crate::pdf::edit;
use crate::pdf::text::{extract_runs, TextRun};
use crate::worker::{Event, Worker};
use egui::{Align2, Color32, Context, FontId, Key, Rect, Rounding, Stroke, TextureHandle, TextureOptions, Vec2};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

const RENDER_DPI: u32 = 150;

/// Text typed into the edit box may carry characters no font can draw.
fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| if c == '\n' || c == '\r' || c == '\t' { ' ' } else { c })
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_string()
}
const OCR_DPI: u32 = 300;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Run(usize),
    OcrWord(usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShapeTool {
    Select,
    Line,
    Rect,
    Circle,
}

struct EditTarget {
    target: Target,
    buffer: String,
    focus: bool,
}

pub struct PdfEditorApp {
    worker: Option<Worker>,
    doc: Option<PdfDocument>,
    path: Option<PathBuf>,
    page: usize,

    geometry: PageGeometry,
    runs: Vec<TextRun>,
    fonts: Vec<FontInfo>,
    scanned: bool,

    generation: u64,
    snapshot: Option<PathBuf>,
    snapshot_gen: u64,

    texture: Option<TextureHandle>,
    texture_key: (usize, u64),
    image_size: Vec2,
    render_pending: bool,
    render_error: Option<String>,

    ocr_words: Vec<OcrWord>,
    ocr_key: (usize, u64),
    ocr_running: bool,
    ocr_lang: String,
    ocr_engine: String,

    zoom: f32,
    pan: Vec2,
    fit_pending: bool,

    editing: Option<EditTarget>,
    hovered: Option<Target>,
    overlay_background: [f32; 3],

    font_ranking: Vec<FontMatch>,
    font_variants: Vec<PathBuf>,
    font_choice: Option<PathBuf>,
    /// Evidence for the document's typeface, summed across every page OCR'd so
    /// far. A scanned document is set in one family; a single ambiguous page
    /// must not override what the clear pages agree on.
    doc_font_votes: std::collections::HashMap<PathBuf, (f32, u32)>,
    doc_family: Option<String>,
    font_sampling_started: bool,
    per_word_font: bool,
    ocr_image: Option<Arc<image::GrayImage>>,
    ocr_image_scale: f32,
    ttf_cache: std::collections::HashMap<PathBuf, Arc<crate::pdf::ttf::TtfFont>>,
    downloading: Option<String>,

    moving: Option<Target>,
    press_target: Option<Target>,
    move_offset_pdf: Vec2,

    page_shapes: HashMap<usize, Vec<crate::pdf::shapes::Shape>>,
    selected_shape: Option<usize>,
    shape_tool: ShapeTool,
    shape_color: [f32; 3],
    shape_fill: bool,
    shape_fill_color: [f32; 3],
    shape_thickness: f32,
    drawing: bool,
    draw_start: egui::Pos2,
    draw_current: egui::Pos2,
    shape_moving: Option<usize>,

    search: String,
    replace: String,
    whole_document: bool,

    status: String,
}

impl Default for PdfEditorApp {
    fn default() -> Self {
        Self::new()
    }
}

impl PdfEditorApp {
    pub fn new() -> Self {
        Self {
            worker: None,
            doc: None,
            path: None,
            page: 0,
            geometry: PageGeometry { x0: 0.0, y0: 0.0, width: 612.0, height: 792.0, rotate: 0 },
            runs: Vec::new(),
            fonts: Vec::new(),
            scanned: false,
            generation: 0,
            snapshot: None,
            snapshot_gen: u64::MAX,
            texture: None,
            texture_key: (usize::MAX, u64::MAX),
            image_size: Vec2::ZERO,
            render_pending: false,
            render_error: None,
            ocr_words: Vec::new(),
            ocr_key: (usize::MAX, u64::MAX),
            ocr_running: false,
            ocr_lang: "fra+eng".to_string(),
            ocr_engine: String::new(),
            zoom: 1.0,
            pan: Vec2::ZERO,
            fit_pending: true,
            editing: None,
            hovered: None,
            overlay_background: [1.0, 1.0, 1.0],
            font_ranking: Vec::new(),
            font_variants: Vec::new(),
            font_choice: None,
            doc_font_votes: std::collections::HashMap::new(),
            doc_family: None,
            font_sampling_started: false,
            per_word_font: true,
            ocr_image: None,
            ocr_image_scale: 1.0,
            ttf_cache: std::collections::HashMap::new(),
            downloading: None,
            moving: None,
            press_target: None,
            move_offset_pdf: Vec2::ZERO,

            page_shapes: HashMap::new(),
            selected_shape: None,
            shape_tool: ShapeTool::Select,
            shape_color: [0.0, 0.0, 0.0],
            shape_fill: false,
            shape_fill_color: [1.0, 0.0, 0.0],
            shape_thickness: 2.0,
            drawing: false,
            draw_start: egui::Pos2::ZERO,
            draw_current: egui::Pos2::ZERO,
            shape_moving: None,

            search: String::new(),
            replace: String::new(),
            whole_document: false,
            status: "Ouvrez un PDF (Ctrl+O)".to_string(),
        }
    }

    pub fn open_path(&mut self, path: PathBuf) {
        self.open(path);
    }

    fn open(&mut self, path: PathBuf) {
        match PdfDocument::open(&path) {
            Ok(doc) => {
                self.status = format!("{} — {} page(s)", path.display(), doc.page_count());
                self.doc = Some(doc);
                self.path = Some(path);
                self.page = 0;
                self.generation = 1;
                self.snapshot_gen = u64::MAX;
                self.texture = None;
                self.texture_key = (usize::MAX, u64::MAX);
                self.ocr_key = (usize::MAX, u64::MAX);
                self.editing = None;
                self.fit_pending = true;
                self.doc_font_votes.clear();
                self.doc_family = None;
                self.font_sampling_started = false;
                self.reload_page();
            }
            Err(e) => self.status = format!("Erreur : {e:#}"),
        }
    }

    fn reload_page(&mut self) {
        let Some(doc) = &self.doc else { return };
        if self.page >= doc.page_count() {
            self.page = doc.page_count().saturating_sub(1);
        }
        let page = self.page;
        self.geometry = doc.geometry(page).unwrap_or(self.geometry);
        self.fonts = doc.fonts(page).unwrap_or_default();

        let page_id = match doc.page_id(page) {
            Ok(id) => id,
            Err(e) => {
                self.status = format!("Erreur : {e}");
                return;
            }
        };
        self.runs = extract_runs(&doc.doc, page_id).unwrap_or_default();

        self.scanned = doc.is_scanned(page);

        self.editing = None;
        self.hovered = None;
        self.moving = None;
        self.press_target = None;
        self.move_offset_pdf = Vec2::ZERO;
        self.selected_shape = None;

        if self.scanned {
            self.ocr_words.clear();
            self.font_ranking.clear();
            self.font_choice = None;
            self.ocr_image = None;
            // Keep the document-level family; only its per-page evidence resets.
            if let Some(family) = self.doc_family.clone() {
                self.font_variants =
                    crate::ocr::fontmatch::family_variants(&family, &crate::fontstore::candidates());
            } else {
                self.font_variants.clear();
            }
            self.request_ocr();
            self.status = format!("Page {} — scannée, OCR en cours", page + 1);
        } else {
            let chars: usize = self.runs.iter().map(|r| r.text.chars().count()).sum();
            self.status = format!(
                "Page {} — {} bloc(s), {} caractères, {} police(s)",
                page + 1,
                self.runs.len(),
                chars,
                self.fonts.len()
            );
        }
        self.request_render();
    }

    fn snapshot_path(&mut self) -> Option<PathBuf> {
        let doc = self.doc.as_mut()?;
        if self.snapshot_gen == self.generation {
            return self.snapshot.clone();
        }
        let dir = std::env::temp_dir().join("pdf_editor_session");
        std::fs::create_dir_all(&dir).ok()?;
        let path = dir.join(format!("gen_{}.pdf", self.generation));
        if doc.write_snapshot(&path).is_err() {
            return None;
        }
        if let Some(old) = self.snapshot.take() {
            let _ = std::fs::remove_file(old);
        }
        self.snapshot = Some(path.clone());
        self.snapshot_gen = self.generation;
        Some(path)
    }

    fn request_render(&mut self) {
        let (Some(path), Some(worker)) = (self.snapshot_path(), self.worker.as_ref()) else {
            return;
        };
        self.render_pending = true;
        self.render_error = None;
        worker.render(path, self.page, RENDER_DPI, self.generation);
    }

    fn request_ocr(&mut self) {
        if self.ocr_key == (self.page, self.generation) {
            return;
        }
        let lang = self.ocr_lang.clone();
        // Match faces at the scan's own resolution, never above it.
        let match_dpi = self
            .doc
            .as_ref()
            .and_then(|d| d.scan_dpi(self.page))
            .unwrap_or(OCR_DPI)
            .clamp(120, 400);
        let (Some(path), Some(worker)) = (self.snapshot_path(), self.worker.as_ref()) else {
            return;
        };
        self.ocr_running = true;
        self.ocr_key = (self.page, self.generation);
        worker.ocr(path, self.page, OCR_DPI, match_dpi, lang, self.generation);
    }

    /// Kicks off OCR on every scanned page so the document's typeface is decided
    /// from all of them, not just the one on screen. Bounded, and the current
    /// page is skipped since it is OCR'd already.
    fn sample_document_fonts(&mut self) {
        let Some(doc) = self.doc.as_ref() else { return };
        let count = doc.page_count().min(8);
        let scanned: Vec<usize> = (0..count)
            .filter(|&p| p != self.page && doc.is_scanned(p))
            .collect();
        let pages: Vec<(usize, u32)> = scanned
            .into_iter()
            .map(|p| {
                let dpi = self
                    .doc
                    .as_ref()
                    .and_then(|d| d.scan_dpi(p))
                    .unwrap_or(OCR_DPI)
                    .clamp(120, 400);
                (p, dpi)
            })
            .collect();
        if pages.is_empty() {
            return;
        }
        let lang = self.ocr_lang.clone();
        let Some(path) = self.snapshot_path() else { return };
        if let Some(worker) = self.worker.as_ref() {
            worker.sample_fonts(path, pages, lang, self.generation);
        }
    }

    /// Folds a page's font ranking into the document-wide tally and re-derives
    /// the winning family and its cuts.
    ///
    /// A font is scored by its *best* page, not its average: the true typeface
    /// stands out only where the scan is clean, and averaging that peak against
    /// noisy pages would bury it under a font that is mediocre everywhere.
    fn record_font_votes(&mut self, ranking: &[FontMatch]) {
        for m in ranking {
            let entry = self.doc_font_votes.entry(m.path.clone()).or_insert((0.0, 0));
            entry.0 = entry.0.max(m.score);
            entry.1 += 1;
        }

        let best = self
            .doc_font_votes
            .iter()
            .map(|(path, (best, _))| (path.clone(), *best))
            .max_by(|a, b| a.1.total_cmp(&b.1));

        // Image matching at scan resolution is weak (a few percent correlation),
        // so it cannot be trusted to pick between look-alike sans-serifs — and a
        // wrong, wider guess (Verdana for an Arial scan) both looks off and makes
        // replacements overflow. Unless a candidate is clearly ahead, fall back
        // to Arial: the most common face for these documents, and narrow enough
        // that a correction never spills past its box.
        let confident = best.as_ref().map(|(_, s)| *s >= 0.75).unwrap_or(false);
        let family = if confident {
            best.and_then(|(p, _)| self.load_ttf(&p)).map(|f| f.family.clone())
        } else {
            self.default_sans_family().or_else(|| {
                best.and_then(|(p, _)| self.load_ttf(&p)).map(|f| f.family.clone())
            })
        };

        if let Some(family) = family {
            if self.doc_family.as_deref() != Some(family.as_str()) {
                self.doc_family = Some(family.clone());
                let candidates = crate::fontstore::candidates();
                self.font_variants =
                    crate::ocr::fontmatch::family_variants(&family, &candidates);
            }
        }
    }

    /// A safe, ubiquitous narrow sans-serif to default scanned text to when the
    /// visual match is not confident.
    fn default_sans_family(&self) -> Option<String> {
        for name in ["arial.ttf", "arimo.ttf", "LiberationSans-Regular.ttf", "helvetica.ttf"] {
            let path = std::path::Path::new(r"C:\Windows\Fonts").join(name);
            if let Ok(font) = crate::pdf::ttf::TtfFont::load(&path) {
                return Some(font.family);
            }
        }
        None
    }

    fn run_rect_image(&self, run: &TextRun) -> Rect {
        let r = crate::pdf::text::pdf_rect_to_image(&self.geometry, RENDER_DPI, run.rect);
        Rect::from_min_max(egui::pos2(r[0], r[1]), egui::pos2(r[2], r[3]))
    }

    fn ocr_rect_image(&self, word: &OcrWord) -> Rect {
        let s = RENDER_DPI as f32 / OCR_DPI as f32;
        Rect::from_min_size(
            egui::pos2(word.bbox.0 * s, word.bbox.1 * s),
            egui::vec2(word.bbox.2 * s, word.bbox.3 * s),
        )
    }

    fn commit_edit(&mut self) {
        let Some(target) = self.editing.take() else { return };
        // A stray newline or tab cannot be drawn by any font: strip it rather
        // than failing the whole edit.
        let buffer = sanitize(&target.buffer);
        match target.target {
            Target::Run(index) => self.commit_run_edit(index, buffer),
            Target::OcrWord(index) => self.commit_ocr_edit(index, buffer),
        }
    }

    /// Replaces the text of a real text run. Returns false and sets the status
    /// on failure. Public so the edit path can be exercised without a window.
    pub fn edit_run(&mut self, index: usize, text: &str) -> bool {
        let before = self.generation;
        self.commit_run_edit(index, sanitize(text));
        self.generation > before
    }

    /// Repaints a scanned word. Returns false and sets the status on failure.
    pub fn edit_ocr_word(&mut self, index: usize, text: &str) -> bool {
        let before = self.generation;
        self.commit_ocr_edit(index, sanitize(text));
        self.generation > before
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    fn commit_run_edit(&mut self, index: usize, buffer: String) {
        let Some(run) = self.runs.get(index).cloned() else { return };
        if buffer == run.text {
            return;
        }
        let Some(doc) = self.doc.as_mut() else { return };
        let Ok(page_id) = doc.page_id(self.page) else { return };

        let before = doc.doc.get_page_content(page_id);
        match edit::set_run_text(
            &mut doc.doc,
            page_id,
            run.op_index,
            &run.font_res,
            run.two_byte,
            &buffer,
        ) {
            Ok(()) => {
                doc.record_undo(page_id, before);
                self.generation += 1;
                self.status = format!("Texte modifié ({})", run.base_font);
                self.refresh_after_edit();
            }
            Err(e) => self.status = format!("Édition impossible : {e:#}"),
        }
    }

    /// Scanned page: cover the scanned word and repaint the corrected text.
    fn commit_ocr_edit(&mut self, index: usize, buffer: String) {
        let Some(word) = self.ocr_words.get(index).cloned() else { return };
        if buffer == word.text || buffer.trim().is_empty() {
            return;
        }
        let geometry = self.geometry;
        let Some(doc) = self.doc.as_mut() else { return };
        let Ok(page_id) = doc.page_id(self.page) else { return };

        let scale = 72.0 / OCR_DPI as f32;
        let (_, visual_height) = crate::pdf::overlay::visual_size(&geometry);
        let to_visual = |w: &OcrWord| {
            [
                w.bbox.0 * scale,
                visual_height - (w.bbox.1 + w.bbox.3) * scale,
                (w.bbox.0 + w.bbox.2) * scale,
                visual_height - w.bbox.1 * scale,
            ]
        };
        let rect = to_visual(&word);

        // Words on the same line, to the right of the edited one: same baseline
        // band, starting further right. They set the room available and, if the
        // replacement is too long even for that room, get pushed along.
        let line_h = rect[3] - rect[1];
        let mid_x = (rect[0] + rect[2]) / 2.0;
        let mut followers: Vec<(f32, usize, OcrWord)> = self
            .ocr_words
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != index)
            .filter_map(|(i, w)| {
                let r = to_visual(w);
                // Same line = the boxes overlap vertically. Robust to a neighbour
                // being a shorter word (a digit next to a word) whose centre sits
                // higher or lower than this one's.
                let overlap = (rect[3].min(r[3]) - rect[1].max(r[1])).max(0.0);
                let same_line = overlap > line_h.min(r[3] - r[1]) * 0.4;
                let to_right = r[0] > mid_x;
                (same_line && to_right).then_some((r[0], i, w.clone()))
            })
            .collect();
        followers.sort_by(|a, b| a.0.total_cmp(&b.0));

        let next_left = followers.first().map(|(x, _, _)| *x);
        // A comfortable gap before the neighbour we do not touch.
        let gap = line_h * 0.35;

        let scan_font = self.font_for_word(index);
        let font_label = scan_font.as_ref().map(|f| f.family.clone()).unwrap_or_else(|| "Helvetica".into());
        let paint_font = match &scan_font {
            Some(font) => crate::pdf::overlay::PaintFont::Embedded(font),
            None => crate::pdf::overlay::PaintFont::Helvetica,
        };

        // How wide would the replacement be, drawn at the line height?
        let needed = match &scan_font {
            Some(font) => {
                let (_, w, _, _) = crate::pdf::overlay::fit_to_box(font, &buffer, line_h);
                w
            }
            None => line_h * 0.55 * buffer.chars().count() as f32,
        };
        let available = next_left.map(|nl| nl - gap - rect[0]).unwrap_or(f32::MAX);

        let Some(doc) = self.doc.as_mut() else { return };
        let before = doc.doc.get_page_content(page_id);
        let mut absorbed: Vec<usize> = Vec::new();

        let result = if next_left.is_none() || needed <= available.max(rect[2] - rect[0]) {
            // Fits before the next word: grow into the blank space, leave the
            // neighbours as scanned ink. Extend the mask only if we overrun the
            // old box, and never past the next word.
            let grown_right = (rect[0] + needed).min(next_left.map(|nl| nl - gap * 0.4).unwrap_or(f32::MAX));
            let mask_right = grown_right.max(rect[2]);
            let word_box = crate::pdf::overlay::WordBox {
                rect: [rect[0], rect[1], mask_right.max(rect[2]), rect[3]],
                text: buffer.clone(),
            };
            crate::pdf::overlay::paint_line(
                &mut doc.doc,
                page_id,
                &geometry,
                &[word_box],
                Some(mask_right),
                self.overlay_background,
                paint_font,
            )
        } else {
            // Too long even for the gap: retypeset the whole tail of the line as
            // one string. Joining the words with real spaces lets the font's own
            // space advance set the gaps — reconstructing pixel gaps from the scan
            // collapsed them the moment the text grew, gluing words together.
            let mut text = buffer.clone();
            let mut last_right = rect[2];
            let mut line_bottom = rect[1];
            let mut line_top = rect[3];
            for (_, _, w) in &followers {
                let r = to_visual(w);
                last_right = r[2];
                line_bottom = line_bottom.min(r[1]);
                line_top = line_top.max(r[3]);
                text.push(' ');
                text.push_str(&w.text);
            }
            // These words are now part of the reflowed run, no longer standalone
            // scanned words: forget their stale boxes so a later edit on the same
            // line cannot repaint over them from the wrong place.
            absorbed = followers.iter().map(|(_, i, _)| *i).collect();
            // One consistent text size for the line: the tallest word's box.
            let word_box = crate::pdf::overlay::WordBox {
                rect: [rect[0], line_bottom, last_right, line_top],
                text,
            };
            crate::pdf::overlay::paint_line(
                &mut doc.doc,
                page_id,
                &geometry,
                std::slice::from_ref(&word_box),
                Some(last_right),
                self.overlay_background,
                paint_font,
            )
        };

        match result {
            Ok(()) => {
                doc.record_undo(page_id, before);
                self.generation += 1;
                if let Some(w) = self.ocr_words.get_mut(index) {
                    w.text = buffer;
                }
                // Drop absorbed followers, highest index first to keep the rest valid.
                absorbed.sort_unstable_by(|a, b| b.cmp(a));
                for i in absorbed {
                    if i < self.ocr_words.len() {
                        self.ocr_words.remove(i);
                    }
                }
                self.status = format!("Zone scannée réécrite en {font_label}");
                self.refresh_after_edit();
            }
            Err(e) => self.status = format!("Réécriture impossible : {e:#}"),
        }
    }

    fn load_ttf(&mut self, path: &std::path::Path) -> Option<Arc<crate::pdf::ttf::TtfFont>> {
        if let Some(font) = self.ttf_cache.get(path) {
            return Some(font.clone());
        }
        match crate::pdf::ttf::TtfFont::load(path) {
            Ok(font) => {
                let font = Arc::new(font);
                self.ttf_cache.insert(path.to_path_buf(), font.clone());
                Some(font)
            }
            Err(e) => {
                self.status = format!("Police inutilisable : {e:#}");
                None
            }
        }
    }

    /// The face to set a correction in. A page rarely has a single font: a bold
    /// heading and the body text sit side by side. So unless the user has forced
    /// a choice, the correction is matched against the *word being edited*,
    /// among the faces that already scored well on the page.
    fn font_for_word(&mut self, index: usize) -> Option<Arc<crate::pdf::ttf::TtfFont>> {
        if let Some(path) = self.font_choice.clone() {
            return self.load_ttf(&path);
        }

        // Within the page family, pick the cut for this word. Body text is
        // regular, so regular is the default: a bold or italic cut is chosen only
        // when it beats the best regular cut by a clear margin. Short and numeric
        // words carry too little signal to trust a narrow win — picking bold on
        // them was turning ordinary text heavy.
        if self.per_word_font && !self.font_variants.is_empty() {
            if let (Some(image), Some(word)) =
                (self.ocr_image.clone(), self.ocr_words.get(index).cloned())
            {
                let word = crate::worker::scale_word(&word, self.ocr_image_scale);
                let ranked = crate::ocr::fontmatch::rank_for_word(&image, &word, &self.font_variants);

                let mut best_regular: Option<(f32, PathBuf)> = None;
                let mut best_styled: Option<(f32, PathBuf)> = None;
                for m in &ranked {
                    if let Some(font) = self.load_ttf(&m.path) {
                        let slot = if font.bold || font.italic {
                            &mut best_styled
                        } else {
                            &mut best_regular
                        };
                        if slot.as_ref().is_none_or(|(s, _)| m.score > *s) {
                            *slot = Some((m.score, m.path.clone()));
                        }
                    }
                }

                let chosen = match (best_regular, best_styled) {
                    (Some((rs, _rp)), Some((ss, sp))) if ss > rs + 0.10 => sp,
                    (Some((_, rp)), _) => rp,
                    (None, Some((_, sp))) => sp,
                    (None, None) => return self.regular_cut(),
                };
                if let Some(font) = self.load_ttf(&chosen) {
                    self.status = format!("Police du mot : {}", font.style_name());
                    return Some(font);
                }
            }
        }

        // Fallback: the regular cut of the document family.
        if let Some(regular) = self.regular_cut() {
            return Some(regular);
        }
        let best = self.font_ranking.first()?.path.clone();
        self.load_ttf(&best)
    }

    /// The upright, regular weight of the document family, if known.
    fn regular_cut(&mut self) -> Option<Arc<crate::pdf::ttf::TtfFont>> {
        let variants = self.font_variants.clone();
        let mut chosen: Option<Arc<crate::pdf::ttf::TtfFont>> = None;
        for path in &variants {
            if let Some(font) = self.load_ttf(path) {
                if !font.bold && !font.italic {
                    return Some(font);
                }
                chosen.get_or_insert(font);
            }
        }
        chosen
    }

    fn commit_move(&mut self) {
        let target = match self.moving.take() {
            Some(t) => t,
            None => return,
        };
        let offset = self.move_offset_pdf;
        self.move_offset_pdf = Vec2::ZERO;

        if offset.x == 0.0 && offset.y == 0.0 {
            return;
        }

        match target {
            Target::Run(index) => {
                let Some(run) = self.runs.get(index) else {
                    self.status = "Bloc introuvable".into();
                    return;
                };
                let Some(doc) = self.doc.as_mut() else { return };
                let Ok(page_id) = doc.page_id(self.page) else { return };

                let before = doc.doc.get_page_content(page_id);
                match crate::pdf::edit::move_run_by(
                    &mut doc.doc,
                    page_id,
                    run.op_index,
                    offset.x,
                    offset.y,
                ) {
                    Ok(()) => {
                        doc.record_undo(page_id, before);
                        self.generation += 1;
                        self.status = format!("Bloc déplacé ({} pt, {} pt)", offset.x, offset.y);
                        self.refresh_after_edit();
                    }
                    Err(e) => self.status = format!("Déplacement impossible : {e:#}"),
                }
            }
            Target::OcrWord(_) => {
                self.status = "Déplacement de mots OCR pas encore supporté".into();
            }
        }
    }

    fn refresh_after_edit(&mut self) {
        let Some(doc) = &self.doc else { return };
        if let Ok(page_id) = doc.page_id(self.page) {
            self.runs = extract_runs(&doc.doc, page_id).unwrap_or_default();
        }
        self.request_render();
    }

    fn replace_all(&mut self) {
        if self.search.is_empty() {
            self.status = "Texte à chercher vide".into();
            return;
        }
        let (search, replace) = (self.search.clone(), self.replace.clone());
        let Some(doc) = self.doc.as_mut() else { return };

        let pages: Vec<usize> = if self.whole_document {
            (0..doc.page_count()).collect()
        } else {
            vec![self.page]
        };

        let mut count = 0usize;
        let mut skipped = 0usize;

        for page in pages {
            let Ok(page_id) = doc.page_id(page) else { continue };
            let runs = extract_runs(&doc.doc, page_id).unwrap_or_default();
            let targets: Vec<&TextRun> = runs.iter().filter(|r| r.text.contains(&search)).collect();
            if targets.is_empty() {
                continue;
            }
            let before = doc.doc.get_page_content(page_id);
            let mut changed = false;

            for run in targets {
                let new_text = run.text.replace(&search, &replace);
                match edit::set_run_text(
                    &mut doc.doc,
                    page_id,
                    run.op_index,
                    &run.font_res,
                    run.two_byte,
                    &new_text,
                ) {
                    Ok(()) => {
                        count += run.text.matches(&search).count();
                        changed = true;
                    }
                    Err(_) => skipped += 1,
                }
            }
            if changed {
                doc.record_undo(page_id, before);
            }
        }

        if count > 0 {
            self.generation += 1;
            self.refresh_after_edit();
        }
        self.status = if skipped > 0 {
            format!("{count} remplacement(s), {skipped} bloc(s) non encodables")
        } else {
            format!("{count} remplacement(s)")
        };
    }

    fn undo(&mut self) {
        let Some(doc) = self.doc.as_mut() else { return };
        if doc.undo().is_some() {
            self.generation += 1;
            self.status = "Annulé".into();
            self.reload_page();
        }
    }

    fn redo(&mut self) {
        let Some(doc) = self.doc.as_mut() else { return };
        if doc.redo().is_some() {
            self.generation += 1;
            self.status = "Rétabli".into();
            self.reload_page();
        }
    }

    /// Writes the edited document to `target`. Public for the same reason.
    pub fn save_document(&mut self, target: &std::path::Path) -> anyhow::Result<()> {
        let doc = self
            .doc
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Aucun document ouvert"))?;
        for (&page, shapes) in &self.page_shapes {
            if !shapes.is_empty() {
                let page_id = doc.page_id(page)?;
                crate::pdf::shapes::append_shapes_to_page(&mut doc.doc, page_id, shapes)?;
            }
        }
        doc.save(target)?;
        self.path = Some(target.to_path_buf());
        self.status = format!("Sauvegardé : {}", target.display());
        Ok(())
    }

    fn save(&mut self, save_as: bool) {
        let target = if save_as || self.path.is_none() {
            let default = self
                .path
                .as_ref()
                .and_then(|p| p.file_stem().map(|s| format!("{}_edite.pdf", s.to_string_lossy())))
                .unwrap_or_else(|| "document.pdf".into());
            rfd::FileDialog::new()
                .add_filter("PDF", &["pdf"])
                .set_file_name(default)
                .save_file()
        } else {
            self.path.clone()
        };

        let Some(target) = target else { return };
        if let Err(e) = self.save_document(&target) {
            self.status = format!("Erreur de sauvegarde : {e:#}");
        }
    }

    fn go_to(&mut self, page: usize) {
        let Some(doc) = &self.doc else { return };
        let page = page.min(doc.page_count().saturating_sub(1));
        if page != self.page {
            self.page = page;
            self.fit_pending = true;
            self.reload_page();
        }
    }

    fn handle_events(&mut self, ctx: &Context) {
        let Some(worker) = &self.worker else { return };
        for event in worker.poll() {
            match event {
                Event::Rendered { page, gen, image } => {
                    if gen != self.generation || page != self.page {
                        continue;
                    }
                    self.image_size = Vec2::new(image.width() as f32, image.height() as f32);
                    self.texture = Some(ctx.load_texture("page", image, TextureOptions::LINEAR));
                    self.texture_key = (page, gen);
                    self.render_pending = false;
                    self.render_error = None;
                }
                Event::RenderFailed { page, gen, error } => {
                    if gen == self.generation && page == self.page {
                        self.render_pending = false;
                        self.render_error = Some(error);
                    }
                }
                Event::OcrDone { page, gen, engine, words, fonts, variants: _, image, image_scale } => {
                    if gen != self.generation {
                        continue;
                    }
                    // Every page's evidence counts toward the document typeface.
                    self.record_font_votes(&fonts);

                    if page != self.page {
                        continue;
                    }
                    self.ocr_running = false;
                    self.ocr_engine = engine;
                    self.status = match &self.doc_family {
                        Some(family) => format!(
                            "{} : {} mot(s) — police du document : {}",
                            self.ocr_engine,
                            words.len(),
                            family
                        ),
                        None => format!("{} : {} mot(s)", self.ocr_engine, words.len()),
                    };
                    self.ocr_words = words;
                    self.font_ranking = fonts;
                    self.ocr_image = Some(image);
                    self.ocr_image_scale = image_scale;
                }
                Event::FontSample { gen, ranking } => {
                    if gen == self.generation {
                        self.record_font_votes(&ranking);
                    }
                }
                Event::Downloaded { family, path } => {
                    self.downloading = None;
                    self.font_choice = Some(path);
                    self.status = format!("{family} téléchargée : elle sera embarquée");
                }
                Event::DownloadFailed { family, error } => {
                    self.downloading = None;
                    self.status = format!("Téléchargement de {family} : {error}");
                }
                Event::OcrFailed { page, gen, error } => {
                    if gen == self.generation && page == self.page {
                        self.ocr_running = false;
                        self.status = format!("OCR : {error}");
                    }
                }
            }
        }
    }
}

impl eframe::App for PdfEditorApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.ui(ctx);
    }
}

impl PdfEditorApp {
    /// One frame of the interface. Separated from the eframe entry point so the
    /// whole interaction can be driven headlessly in tests.
    pub fn ui(&mut self, ctx: &Context) {
        if self.worker.is_none() {
            self.worker = Some(Worker::spawn(ctx.clone()));
        }
        self.handle_events(ctx);

        if self.doc.is_some()
            && self.texture_key != (self.page, self.generation)
            && !self.render_pending
            && self.render_error.is_none()
        {
            self.request_render();
            if self.scanned {
                self.request_ocr();
            }
        }

        // Sample every page's typeface once the worker exists (it does not yet
        // when the file is opened from the command line).
        if self.worker.is_some() && self.doc.is_some() && !self.font_sampling_started {
            self.font_sampling_started = true;
            self.sample_document_fonts();
        }

        let typing = self.editing.is_some();

        let (ctrl_o, ctrl_s, ctrl_z, ctrl_y, ctrl_0, next, prev, escape) = ctx.input(|i| {
            (
                i.modifiers.ctrl && i.key_pressed(Key::O),
                i.modifiers.ctrl && i.key_pressed(Key::S),
                i.modifiers.ctrl && i.key_pressed(Key::Z),
                i.modifiers.ctrl && i.key_pressed(Key::Y),
                i.modifiers.ctrl && i.key_pressed(Key::Num0),
                i.key_pressed(Key::PageDown),
                i.key_pressed(Key::PageUp),
                i.key_pressed(Key::Escape),
            )
        });

        if ctrl_o {
            if let Some(p) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).pick_file() {
                self.open(p);
            }
        }

        if self.doc.is_some() {
            if ctrl_s {
                self.save(false);
            }
            if ctrl_z && !typing {
                self.undo();
            }
            if ctrl_y && !typing {
                self.redo();
            }
            if ctrl_0 {
                self.fit_pending = true;
            }
            if next && !typing {
                self.go_to(self.page + 1);
            }
            if prev && !typing && self.page > 0 {
                self.go_to(self.page - 1);
            }
            if escape {
                self.editing = None;
            }
            if ctx.input(|i| i.key_pressed(Key::Delete)) && !typing {
                if let Some(idx) = self.selected_shape {
                    if let Some(shapes) = self.page_shapes.get_mut(&self.page) {
                        if idx < shapes.len() {
                            shapes.remove(idx);
                            self.selected_shape = None;
                            self.status = "Forme supprimée".into();
                        }
                    }
                }
            }
        }

        self.top_bar(ctx);

        if self.doc.is_none() {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(120.0);
                    ui.heading("Éditeur PDF");
                    ui.label("Ouvrez un PDF pour lire et éditer son texte.");
                    ui.add_space(8.0);
                    if ui.button("Ouvrir un PDF").clicked() {
                        if let Some(p) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).pick_file() {
                            self.open(p);
                        }
                    }
                    ui.add_space(16.0);
                    ui.label(format!("Moteur de rendu : {}", crate::pdf::render::renderer_name()));
                    if !crate::pdf::render::is_renderer_available() {
                        ui.colored_label(Color32::RED, "Ghostscript / mutool / pdftoppm requis pour l'affichage");
                    }
                });
            });
            return;
        }

        self.side_panel(ctx);
        self.page_view(ctx);
    }

    pub fn next_page(&mut self) {
        self.go_to(self.page + 1);
    }

    pub fn ocr_word_count(&self) -> usize {
        self.ocr_words.len()
    }

    pub fn font_ranking(&self) -> Vec<(String, f32)> {
        self.font_ranking.iter().map(|m| (m.family.clone(), m.score)).collect()
    }

    pub fn doc_family(&self) -> Option<String> { self.doc_family.clone() }

    pub fn current_page(&self) -> usize {
        self.page
    }

    pub fn ocr_word_bbox(&self, index: usize) -> Option<(f32, f32, f32, f32)> {
        self.ocr_words.get(index).map(|w| w.bbox)
    }

    pub fn geometry(&self) -> PageGeometry {
        self.geometry
    }

    pub fn ocr_word_text(&self, index: usize) -> Option<&str> {
        self.ocr_words.get(index).map(|w| w.text.as_str())
    }

    pub fn is_scanned(&self) -> bool {
        self.scanned
    }

    fn top_bar(&mut self, ctx: &Context) {
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.menu_button("Fichier", |ui| {
                    if ui.button("Ouvrir…  Ctrl+O").clicked() {
                        if let Some(p) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).pick_file() {
                            self.open(p);
                        }
                        ui.close_menu();
                    }
                    if self.doc.is_some() {
                        if ui.button("Enregistrer  Ctrl+S").clicked() {
                            self.save(false);
                            ui.close_menu();
                        }
                        if ui.button("Enregistrer sous…").clicked() {
                            self.save(true);
                            ui.close_menu();
                        }
                    }
                    if ui.button("Quitter").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                if let Some(doc) = &self.doc {
                    let count = doc.page_count();
                    let can_undo = doc.can_undo();
                    let can_redo = doc.can_redo();

                    ui.separator();
                    if ui.add_enabled(self.page > 0, egui::Button::new("◀")).clicked() {
                        self.go_to(self.page.saturating_sub(1));
                    }
                    ui.label(format!("{} / {}", self.page + 1, count));
                    if ui.add_enabled(self.page + 1 < count, egui::Button::new("▶")).clicked() {
                        self.go_to(self.page + 1);
                    }

                    ui.separator();
                    if ui.add_enabled(can_undo, egui::Button::new("↶")).on_hover_text("Annuler (Ctrl+Z)").clicked() {
                        self.undo();
                    }
                    if ui.add_enabled(can_redo, egui::Button::new("↷")).on_hover_text("Rétablir (Ctrl+Y)").clicked() {
                        self.redo();
                    }

                    ui.separator();
                    if ui.button("−").clicked() {
                        self.zoom = (self.zoom / 1.2).clamp(0.05, 16.0);
                    }
                    ui.label(format!("{:.0}%", self.zoom * 100.0));
                    if ui.button("+").clicked() {
                        self.zoom = (self.zoom * 1.2).clamp(0.05, 16.0);
                    }
                    if ui.button("Ajuster").clicked() {
                        self.fit_pending = true;
                    }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.render_pending {
                        ui.spinner();
                    }
                    let failed = self.status.contains("impossible")
                        || self.status.contains("Erreur")
                        || self.status.contains("Caractères");
                    if failed {
                        ui.colored_label(Color32::from_rgb(230, 90, 90), &self.status);
                    } else {
                        ui.label(&self.status);
                    }
                    if self.doc.as_ref().map(|d| d.dirty).unwrap_or(false) {
                        ui.colored_label(
                            Color32::from_rgb(240, 180, 60),
                            "● modifié — Ctrl+S pour enregistrer",
                        );
                    }
                });
            });
        });
    }

    fn side_panel(&mut self, ctx: &Context) {
        egui::SidePanel::left("side")
            .resizable(true)
            .default_width(360.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(4.0);
                    ui.heading("Rechercher & remplacer");
                    ui.horizontal(|ui| {
                        ui.label("Chercher");
                        ui.text_edit_singleline(&mut self.search);
                    });
                    ui.horizontal(|ui| {
                        ui.label("Remplacer");
                        ui.text_edit_singleline(&mut self.replace);
                    });
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut self.whole_document, "Tout le document");
                        if ui.button("Remplacer tout").clicked() {
                            self.replace_all();
                        }
                    });

                    ui.separator();
                    ui.heading("Polices de la page");
                    if self.fonts.is_empty() {
                        ui.weak("Aucune police (page image)");
                    }
                    for font in &self.fonts {
                        ui.horizontal_wrapped(|ui| {
                            ui.monospace(format!("/{}", font.internal_name));
                            ui.colored_label(Color32::from_rgb(90, 160, 255), &font.base_font);
                            ui.weak(&font.font_type);
                            if font.embedded {
                                ui.colored_label(Color32::from_rgb(80, 190, 120), "intégrée");
                            }
                            if font.coverage > 0 {
                                ui.colored_label(
                                    Color32::from_rgb(80, 190, 120),
                                    format!("{} · {} glyphes", font.map_source, font.coverage),
                                )
                                .on_hover_text("Table de caractères utilisée pour réécrire le texte");
                            } else {
                                ui.colored_label(Color32::from_rgb(220, 90, 90), "non réencodable");
                            }
                        });
                    }

                    ui.separator();
                    if self.scanned {
                        ui.heading("OCR");
                        ui.horizontal(|ui| {
                            ui.label("Langues");
                            ui.text_edit_singleline(&mut self.ocr_lang);
                            if ui.button("Relancer").clicked() {
                                self.ocr_key = (usize::MAX, u64::MAX);
                                self.request_ocr();
                            }
                        });
                        if self.ocr_running {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label("Analyse en cours…");
                            });
                        }
                        ui.separator();
                        ui.heading("Police du scan");
                        if self.font_ranking.is_empty() {
                            if self.ocr_running {
                                ui.weak("Analyse de la police en cours…");
                            } else {
                                ui.colored_label(
                                    Color32::from_rgb(230, 170, 60),
                                    "Police non identifiée : Helvetica sera utilisée.",
                                );
                            }
                        } else {
                            ui.checkbox(
                                &mut self.per_word_font,
                                "Police adaptée à chaque mot (recommandé)",
                            )
                            .on_hover_text(
                                "Une page mêle souvent plusieurs graisses : la police est \
                                 alors choisie sur le mot édité, pas sur la moyenne de la page.",
                            );

                            let auto = self.font_choice.is_none();
                            if ui.selectable_label(auto, "Automatique").clicked() {
                                self.font_choice = None;
                            }

                            let ranking: Vec<(std::path::PathBuf, String, f32)> = self
                                .font_ranking
                                .iter()
                                .take(6)
                                .map(|m| (m.path.clone(), m.family.clone(), m.score))
                                .collect();

                            for (path, family, score) in ranking {
                                let selected = self.font_choice.as_ref() == Some(&path);
                                let label = format!("{family}  {:.0}%", score * 100.0);
                                if ui.selectable_label(selected, label).clicked() {
                                    self.font_choice = Some(path);
                                }
                            }
                        }

                        egui::CollapsingHeader::new("Télécharger une police libre")
                            .id_source("font_store")
                            .show(ui, |ui| {
                                ui.weak(
                                    "Polices sous licence ouverte, embarquables légalement. \
                                     Stockées dans votre cache utilisateur, sans installation système.",
                                );
                                for (i, entry) in crate::fontstore::CATALOG.iter().enumerate() {
                                    ui.horizontal_wrapped(|ui| {
                                        let cached = crate::fontstore::is_cached(entry);
                                        ui.label(entry.family);
                                        ui.weak(entry.note);
                                        if cached {
                                            if ui.button("Utiliser").clicked() {
                                                self.font_choice = Some(
                                                    crate::fontstore::cache_dir().join(entry.file),
                                                );
                                            }
                                        } else if self.downloading.as_deref() == Some(entry.family) {
                                            ui.spinner();
                                        } else if ui.button("Télécharger").clicked() {
                                            if let Some(worker) = &self.worker {
                                                self.downloading =
                                                    Some(entry.family.to_string());
                                                self.status =
                                                    format!("Téléchargement de {}…", entry.family);
                                                worker.download_font(i);
                                            }
                                        }
                                    });
                                }
                            });

                        ui.separator();
                        ui.colored_label(
                            Color32::from_rgb(120, 200, 140),
                            "Cliquez un mot vert dans la page pour le corriger.",
                        );
                        let text: String = self
                            .ocr_words
                            .iter()
                            .map(|w| w.text.as_str())
                            .collect::<Vec<_>>()
                            .join(" ");
                        if !text.is_empty() {
                            egui::ScrollArea::vertical()
                                .id_source("ocr_text")
                                .max_height(260.0)
                                .show(ui, |ui| {
                                    ui.label(text);
                                });
                        }
                    } else {
                        ui.heading(format!("Blocs de texte ({})", self.runs.len()));
                        ui.weak("Cliquez un bloc dans la page pour l'éditer.");
                        let runs: Vec<(usize, String, bool)> = self
                            .runs
                            .iter()
                            .enumerate()
                            .map(|(i, r)| (i, r.text.clone(), r.editable))
                            .collect();
                        egui::ScrollArea::vertical()
                            .id_source("runs")
                            .max_height(420.0)
                            .show(ui, |ui| {
                                for (i, text, editable) in runs {
                                    let selected =
                                        self.editing.as_ref().map(|e| e.target) == Some(Target::Run(i));
                                    let label = if text.chars().count() > 60 {
                                        format!("{}…", text.chars().take(60).collect::<String>())
                                    } else {
                                        text.clone()
                                    };
                                    let mut response = ui.selectable_label(selected, label);
                                    if !editable {
                                        response = response.on_hover_text("Police non réencodable");
                                    }
                                    if response.clicked() && editable {
                                        self.editing = Some(EditTarget {
                                            target: Target::Run(i),
                                            buffer: text,
                                            focus: true,
                                        });
                                    }
                                }
                            });
                    }

                    ui.separator();
                    ui.heading("Formes");
                    ui.horizontal(|ui| {
                        let tools = [("Sélect", ShapeTool::Select), ("Ligne", ShapeTool::Line), ("Rect", ShapeTool::Rect), ("Cercle", ShapeTool::Circle)];
                        for (label, tool) in tools {
                            let selected = self.shape_tool == tool;
                            if ui.selectable_label(selected, label).clicked() {
                                self.shape_tool = tool;
                                self.selected_shape = None;
                            }
                        }
                    });

                    if self.shape_tool != ShapeTool::Select {
                        ui.horizontal(|ui| {
                            ui.label("Couleur RVB");
                            ui.color_edit_button_rgb(&mut self.shape_color);
                        });
                        ui.add(
                            egui::Slider::new(&mut self.shape_thickness, 0.5..=50.0)
                                .text("Épaisseur")
                        );
                        ui.checkbox(&mut self.shape_fill, "Remplissage");
                        if self.shape_fill {
                            ui.horizontal(|ui| {
                                ui.label("Couleur rempl.");
                                ui.color_edit_button_rgb(&mut self.shape_fill_color);
                            });
                        }
                    }

                    if let Some(idx) = self.selected_shape {
                        if let Some(shapes) = self.page_shapes.get_mut(&self.page) {
                            if idx < shapes.len() {
                                let deleted = ui.button("Supprimer (Suppr)").clicked();
                                if deleted {
                                    shapes.remove(idx);
                                    self.selected_shape = None;
                                } else if let Some(shape) = shapes.get_mut(idx) {
                                    ui.separator();
                                    ui.heading("Forme sélectionnée");
                                    ui.horizontal(|ui| {
                                        ui.label("Couleur");
                                        ui.color_edit_button_rgb(&mut shape.color);
                                    });
                                    ui.add(
                                        egui::Slider::new(&mut shape.thickness, 0.5..=50.0)
                                            .text("Épaisseur")
                                    );
                                    ui.checkbox(&mut shape.fill, "Remplissage");
                                    if shape.fill {
                                        ui.horizontal(|ui| {
                                            ui.label("Couleur rempl.");
                                            ui.color_edit_button_rgb(&mut shape.fill_color);
                                        });
                                    }
                                }
                            }
                        }
                    }
                });
            });
    }

    fn finish_shape(&mut self, origin: egui::Pos2) {
        let start = self.draw_start;
        let end = self.draw_current;
        let to_pdf = |p: egui::Pos2| -> (f32, f32) {
            let ix = (p.x - origin.x) / self.zoom;
            let iy = (p.y - origin.y) / self.zoom;
            crate::pdf::text::image_point_to_pdf(&self.geometry, RENDER_DPI, ix, iy)
        };
        let (x1, y1) = to_pdf(start);
        let (x2, y2) = to_pdf(end);

        let kind = match self.shape_tool {
            ShapeTool::Line => crate::pdf::shapes::ShapeKind::Line { x1, y1, x2, y2 },
            ShapeTool::Rect => {
                let x = x1.min(x2);
                let y = y1.min(y2);
                let w = (x2 - x1).abs();
                let h = (y2 - y1).abs();
                if w < 2.0 && h < 2.0 { return; }
                crate::pdf::shapes::ShapeKind::Rect { x, y, w, h }
            }
            ShapeTool::Circle => {
                let dx = x2 - x1;
                let dy = y2 - y1;
                let radius = (dx * dx + dy * dy).sqrt();
                if radius < 2.0 { return; }
                crate::pdf::shapes::ShapeKind::Circle { cx: x1, cy: y1, radius }
            }
            ShapeTool::Select => return,
        };

        let shapes = self.page_shapes.entry(self.page).or_default();
        shapes.push(crate::pdf::shapes::Shape {
            kind,
            color: self.shape_color,
            fill: self.shape_fill,
            fill_color: self.shape_fill_color,
            thickness: self.shape_thickness,
        });
        self.selected_shape = Some(shapes.len() - 1);
    }

    fn hit_test_shapes(&self, px: f32, py: f32) -> Option<usize> {
        let shapes = self.page_shapes.get(&self.page)?;
        let threshold = 6.0 / (RENDER_DPI as f32 / 72.0);
        for (i, shape) in shapes.iter().enumerate().rev() {
            if shape.kind.hit_test(px, py, threshold) {
                return Some(i);
            }
        }
        None
    }

    fn pdf_point_from_screen(&self, p: egui::Pos2, origin: egui::Pos2) -> (f32, f32) {
        let ix = (p.x - origin.x) / self.zoom;
        let iy = (p.y - origin.y) / self.zoom;
        crate::pdf::text::image_point_to_pdf(&self.geometry, RENDER_DPI, ix, iy)
    }

    fn draw_shapes_on_canvas(&self, painter: &egui::Painter, origin: egui::Pos2) {
        let Some(shapes) = self.page_shapes.get(&self.page) else { return };
        let to_screen = |pdf_x: f32, pdf_y: f32| -> egui::Pos2 {
            let (ix, iy) = crate::pdf::text::pdf_point_to_image(&self.geometry, RENDER_DPI, pdf_x, pdf_y);
            egui::pos2(origin.x + ix * self.zoom, origin.y + iy * self.zoom)
        };

        for (i, shape) in shapes.iter().enumerate() {
            let selected = self.selected_shape == Some(i);
            let moving = self.shape_moving == Some(i);
            let alpha: u8 = if moving { 160 } else { 200 };
            match &shape.kind {
                crate::pdf::shapes::ShapeKind::Line { x1, y1, x2, y2 } => {
                    let a = to_screen(*x1, *y1);
                    let b = to_screen(*x2, *y2);
                    let stroke = egui::Stroke::new(
                        shape.thickness.max(if selected { 2.0 } else { 1.0 }),
                        Color32::from_rgba_premultiplied(
                            (shape.color[0] * 255.0) as u8,
                            (shape.color[1] * 255.0) as u8,
                            (shape.color[2] * 255.0) as u8,
                            alpha,
                        ),
                    );
                    painter.line_segment([a, b], stroke);
                    if selected {
                        let r = 4.0;
                        painter.circle_filled(a, r, Color32::WHITE);
                        painter.circle_stroke(a, r, egui::Stroke::new(1.5, Color32::BLUE));
                        painter.circle_filled(b, r, Color32::WHITE);
                        painter.circle_stroke(b, r, egui::Stroke::new(1.5, Color32::BLUE));
                    }
                }
                crate::pdf::shapes::ShapeKind::Rect { x, y, w, h } => {
                    let a = to_screen(*x, *y);
                    let b = to_screen(*x + *w, *y + *h);
                    let rect = egui::Rect::from_two_pos(a, b);
                    let color = Color32::from_rgba_premultiplied(
                        (shape.color[0] * 255.0) as u8,
                        (shape.color[1] * 255.0) as u8,
                        (shape.color[2] * 255.0) as u8,
                        alpha,
                    );
                    if shape.fill {
                        let fill_color = Color32::from_rgba_premultiplied(
                            (shape.fill_color[0] * 255.0) as u8,
                            (shape.fill_color[1] * 255.0) as u8,
                            (shape.fill_color[2] * 255.0) as u8,
                            100,
                        );
                        painter.rect_filled(rect, Rounding::ZERO, fill_color);
                    }
                    painter.rect_stroke(rect, Rounding::ZERO, egui::Stroke::new(shape.thickness.max(1.0), color));
                    if selected {
                        for corner in [a, egui::pos2(b.x, a.y), b, egui::pos2(a.x, b.y)] {
                            let r = 4.0;
                            painter.circle_filled(corner, r, Color32::WHITE);
                            painter.circle_stroke(corner, r, egui::Stroke::new(1.5, Color32::BLUE));
                        }
                    }
                }
                crate::pdf::shapes::ShapeKind::Circle { cx, cy, radius } => {
                    let center = to_screen(*cx, *cy);
                    let edge = to_screen(*cx + *radius, *cy);
                    let screen_radius = (edge.x - center.x).abs();
                    let color = Color32::from_rgba_premultiplied(
                        (shape.color[0] * 255.0) as u8,
                        (shape.color[1] * 255.0) as u8,
                        (shape.color[2] * 255.0) as u8,
                        alpha,
                    );
                    if shape.fill {
                        let fill_color = Color32::from_rgba_premultiplied(
                            (shape.fill_color[0] * 255.0) as u8,
                            (shape.fill_color[1] * 255.0) as u8,
                            (shape.fill_color[2] * 255.0) as u8,
                            100,
                        );
                        painter.circle_filled(center, screen_radius, fill_color);
                    }
                    painter.circle_stroke(center, screen_radius, egui::Stroke::new(shape.thickness.max(1.0), color));
                    if selected {
                        painter.circle_filled(center, 4.0, Color32::WHITE);
                        painter.circle_stroke(center, 4.0, egui::Stroke::new(1.5, Color32::BLUE));
                    }
                }
            }
        }
    }

    fn page_view(&mut self, ctx: &Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(err) = self.render_error.clone() {
                ui.colored_label(Color32::RED, err);
                if ui.button("Réessayer").clicked() {
                    self.request_render();
                }
                return;
            }

            let Some(texture) = self.texture.clone() else {
                ui.centered_and_justified(|ui| {
                    ui.spinner();
                });
                return;
            };

            let avail = ui.available_rect_before_wrap();
            if self.fit_pending && self.image_size.x > 0.0 {
                self.zoom = ((avail.width() - 24.0) / self.image_size.x).clamp(0.05, 16.0);
                let display = self.image_size * self.zoom;
                self.pan = Vec2::new(((avail.width() - display.x) / 2.0).max(0.0), 8.0);
                self.fit_pending = false;
            }

            let pointer = ctx.pointer_hover_pos();
            let scroll = ctx.input(|i| i.raw_scroll_delta.y);
            let zoom_mod = ctx.input(|i| i.modifiers.ctrl);

            if zoom_mod && scroll.abs() > 0.0 && avail.contains(pointer.unwrap_or_default()) {
                let old = self.zoom;
                let new = (old * (1.0 + scroll * 0.0015)).clamp(0.05, 16.0);
                if let Some(p) = pointer {
                    let origin = avail.min + self.pan;
                    let img_pos = (p - origin) / old;
                    self.pan = p - avail.min - img_pos * new;
                }
                self.zoom = new;
            } else if scroll.abs() > 0.0 {
                self.pan.y += scroll;
            }

            let origin = avail.min + self.pan;
            let display = self.image_size * self.zoom;
            let image_rect = Rect::from_min_size(origin, display);

            // Shape drawing mode (Line/Rect/Circle tools)
            if self.shape_tool != ShapeTool::Select && self.doc.is_some() {
                let mouse_down = ctx.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
                if mouse_down && !self.drawing {
                    if let Some(pos) = pointer {
                        if avail.contains(pos) {
                            self.drawing = true;
                            self.draw_start = pos;
                            self.draw_current = pos;
                        }
                    }
                }
                if self.drawing {
                    if let Some(pos) = pointer {
                        self.draw_current = pos;
                    }
                    if !mouse_down {
                        self.finish_shape(origin);
                        self.drawing = false;
                    }
                }
            }

            let response = ui.allocate_rect(avail, egui::Sense::click_and_drag());

            if response.dragged() {
                if self.shape_moving.is_some() {
                    let delta = response.drag_delta();
                    let s = RENDER_DPI as f32 / 72.0;
                    if let Some(idx) = self.shape_moving {
                        if let Some(shapes) = self.page_shapes.get_mut(&self.page) {
                            if let Some(shape) = shapes.get_mut(idx) {
                                shape.kind.move_by(delta.x / self.zoom / s, -delta.y / self.zoom / s);
                            }
                        }
                    }
                } else if self.moving.is_some() {
                    let delta = response.drag_delta();
                    let s = RENDER_DPI as f32 / 72.0;
                    self.move_offset_pdf += Vec2::new(
                        delta.x / self.zoom / s,
                        -delta.y / self.zoom / s,
                    );
                } else if let Some(target) = self.press_target {
                    self.moving = Some(target);
                    self.move_offset_pdf = Vec2::ZERO;
                    self.commit_edit();
                    let delta = response.drag_delta();
                    if delta != Vec2::ZERO {
                        let s = RENDER_DPI as f32 / 72.0;
                        self.move_offset_pdf += Vec2::new(
                            delta.x / self.zoom / s,
                            -delta.y / self.zoom / s,
                        );
                    }
                } else if self.shape_tool == ShapeTool::Select && self.doc.is_some() {
                    if let Some(pos) = pointer {
                        let (px, py) = self.pdf_point_from_screen(pos, origin);
                        if let Some(idx) = self.hit_test_shapes(px, py) {
                            self.shape_moving = Some(idx);
                            self.selected_shape = Some(idx);
                            let delta = response.drag_delta();
                            if delta != Vec2::ZERO {
                                let s = RENDER_DPI as f32 / 72.0;
                                if let Some(shapes) = self.page_shapes.get_mut(&self.page) {
                                    if let Some(shape) = shapes.get_mut(idx) {
                                        shape.kind.move_by(delta.x / self.zoom / s, -delta.y / self.zoom / s);
                                    }
                                }
                            }
                        } else {
                            self.pan += response.drag_delta();
                        }
                    } else {
                        self.pan += response.drag_delta();
                    }
                } else {
                    self.pan += response.drag_delta();
                }
            }
            if response.drag_stopped() {
                if self.shape_moving.is_some() {
                    self.shape_moving = None;
                }
                if self.moving.is_some() {
                    self.commit_move();
                }
            }

            let painter = ui.painter_at(avail);
            painter.image(
                texture.id(),
                image_rect,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );

            // Hit-testing: real text runs first, OCR words underneath them.
            let to_screen = |r: Rect| {
                Rect::from_min_max(
                    origin + r.min.to_vec2() * self.zoom,
                    origin + r.max.to_vec2() * self.zoom,
                )
            };

            self.hovered = None;
            if let Some(p) = pointer {
                for (i, run) in self.runs.iter().enumerate() {
                    if to_screen(self.run_rect_image(run)).contains(p) {
                        self.hovered = Some(Target::Run(i));
                        break;
                    }
                }
                if self.hovered.is_none() && self.scanned {
                    for (i, word) in self.ocr_words.iter().enumerate() {
                        if to_screen(self.ocr_rect_image(word)).contains(p) {
                            self.hovered = Some(Target::OcrWord(i));
                            break;
                        }
                    }
                }
            }

            let primary_pressed = ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
            let primary_down = ctx.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
            if primary_pressed {
                self.press_target = self.hovered;
            } else if !primary_down {
                self.press_target = None;
            }

            let active = self.editing.as_ref().map(|e| e.target);

            let to_img_offset = |off: Vec2| -> Vec2 {
                let s = RENDER_DPI as f32 / 72.0;
                Vec2::new(off.x * s * self.zoom, -off.y * s * self.zoom)
            };

            for (i, run) in self.runs.iter().enumerate() {
                let target = Target::Run(i);
                let editing = active == Some(target);
                let hovered = self.hovered == Some(target);
                let moving = self.moving == Some(target);
                if !editing && !hovered && !moving {
                    continue;
                }
                let (fill, stroke) = if editing {
                    (
                        Color32::from_rgba_premultiplied(255, 190, 60, 40),
                        Color32::from_rgb(230, 150, 30),
                    )
                } else if moving {
                    (
                        Color32::from_rgba_premultiplied(100, 200, 255, 50),
                        Color32::from_rgb(60, 180, 230),
                    )
                } else if run.editable {
                    (
                        Color32::from_rgba_premultiplied(60, 140, 255, 35),
                        Color32::from_rgb(50, 120, 220),
                    )
                } else {
                    (
                        Color32::from_rgba_premultiplied(255, 80, 80, 30),
                        Color32::from_rgb(200, 70, 70),
                    )
                };
                let mut screen = to_screen(self.run_rect_image(run));
                if moving {
                    screen = screen.translate(to_img_offset(self.move_offset_pdf));
                }
                painter.rect_filled(screen, Rounding::same(2.0), fill);
                painter.rect_stroke(screen, Rounding::same(2.0), Stroke::new(1.0, stroke));
            }

            if self.scanned {
                for (i, word) in self.ocr_words.iter().enumerate() {
                    let target = Target::OcrWord(i);
                    let editing = active == Some(target);
                    let hovered = self.hovered == Some(target);
                    if !editing && !hovered {
                        continue;
                    }
                    let (fill, stroke) = if editing {
                        (
                            Color32::from_rgba_premultiplied(255, 190, 60, 40),
                            Color32::from_rgb(230, 150, 30),
                        )
                    } else {
                        (
                            Color32::from_rgba_premultiplied(120, 200, 140, 40),
                            Color32::from_rgb(60, 170, 100),
                        )
                    };
                    let screen = to_screen(self.ocr_rect_image(word));
                    painter.rect_filled(screen, Rounding::same(2.0), fill);
                    painter.rect_stroke(screen, Rounding::same(2.0), Stroke::new(1.0, stroke));
                }
            }

            // Shape preview during drawing
            if self.drawing {
                let start = self.draw_start;
                let end = self.draw_current;
                let gray = Color32::from_gray(140);
                match self.shape_tool {
                    ShapeTool::Line => {
                        painter.line_segment([start, end], Stroke::new(1.5, gray));
                    }
                    ShapeTool::Rect => {
                        let r = Rect::from_two_pos(start, end);
                        painter.rect_stroke(r, Rounding::ZERO, Stroke::new(1.5, gray));
                    }
                    ShapeTool::Circle => {
                        let dx = end.x - start.x;
                        let dy = end.y - start.y;
                        let radius = (dx * dx + dy * dy).sqrt();
                        painter.circle_stroke(start, radius, Stroke::new(1.5, gray));
                    }
                    _ => {}
                }
            }

            // Draw shapes on canvas
            self.draw_shapes_on_canvas(&painter, origin);

            if response.clicked() {
                if self.shape_tool == ShapeTool::Select && self.doc.is_some() {
                    if let Some(pos) = pointer {
                        let (px, py) = self.pdf_point_from_screen(pos, origin);
                        if let Some(idx) = self.hit_test_shapes(px, py) {
                            self.selected_shape = Some(idx);
                            return;
                        }
                    }
                }
                self.commit_edit();
                match self.hovered {
                    Some(Target::Run(i)) => {
                        if self.runs[i].editable {
                            self.editing = Some(EditTarget {
                                target: Target::Run(i),
                                buffer: self.runs[i].text.clone(),
                                focus: true,
                            });
                        } else {
                            self.status = format!(
                                "Bloc non éditable : {} n'expose aucune table de caractères",
                                self.runs[i].base_font
                            );
                        }
                    }
                    Some(Target::OcrWord(i)) => {
                        self.editing = Some(EditTarget {
                            target: Target::OcrWord(i),
                            buffer: self.ocr_words[i].text.clone(),
                            focus: true,
                        });
                    }
                    None => {}
                }
            }

            painter.text(
                avail.left_top() + Vec2::new(10.0, 8.0),
                Align2::LEFT_TOP,
                format!("{:.0}%", self.zoom * 100.0),
                FontId::proportional(13.0),
                Color32::from_gray(160),
            );

            self.edit_popup(ctx, origin);
        });
    }

    fn edit_popup(&mut self, ctx: &Context, origin: egui::Pos2) {
        let Some(edit_target) = self.editing.as_ref() else { return };
        let target = edit_target.target;

        let (rect, title, detail) = match target {
            Target::Run(i) => {
                let Some(run) = self.runs.get(i) else {
                    self.editing = None;
                    return;
                };
                (
                    self.run_rect_image(run),
                    run.base_font.clone(),
                    format!("{:.1} pt \u{2022} {}", run.font_size, run.map_source),
                )
            }
            Target::OcrWord(i) => {
                let Some(word) = self.ocr_words.get(i) else {
                    self.editing = None;
                    return;
                };
                (
                    self.ocr_rect_image(word),
                    "Zone scannée".to_string(),
                    format!("OCR {:.0}% \u{2022} réécrit en Helvetica", word.confidence.min(100.0)),
                )
            }
        };

        let anchor = origin + Vec2::new(rect.min.x * self.zoom, rect.max.y * self.zoom + 6.0);
        let mut buffer = edit_target.buffer.clone();
        let focus = edit_target.focus;
        let mut commit = false;
        let mut cancel = false;

        egui::Area::new("edit_popup".into())
            .fixed_pos(anchor)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_max_width(460.0);
                    ui.horizontal(|ui| {
                        ui.colored_label(Color32::from_rgb(90, 160, 255), title);
                        ui.weak(detail);
                    });
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut buffer).desired_width(440.0),
                    );
                    if focus {
                        edit.request_focus();
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Appliquer").clicked() {
                            commit = true;
                        }
                        if ui.button("Annuler").clicked() {
                            cancel = true;
                        }
                        ui.weak("Entrée : appliquer \u{2022} Échap : annuler");
                    });
                    if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        commit = true;
                    }
                });
            });

        if let Some(t) = self.editing.as_mut() {
            t.buffer = buffer;
            t.focus = false;
        }
        if commit {
            self.commit_edit();
        } else if cancel {
            self.editing = None;
        }
    }
}
