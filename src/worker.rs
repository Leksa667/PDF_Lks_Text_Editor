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
use egui::{Color32, ColorImage};
use image::GrayImage;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

pub enum Job {
    Render { pdf: PathBuf, page: usize, dpi: u32, gen: u64 },
    Ocr { pdf: PathBuf, page: usize, dpi: u32, match_dpi: u32, lang: String, gen: u64 },
    /// Font-only sampling of several pages, kept as one job so the newest-wins
    /// draining never collapses it to a single page.
    SampleFonts { pdf: PathBuf, pages: Vec<(usize, u32)>, lang: String, gen: u64 },
}

pub enum Event {
    Rendered { page: usize, gen: u64, image: ColorImage },
    RenderFailed { page: usize, gen: u64, error: String },
    OcrDone {
        page: usize,
        gen: u64,
        engine: String,
        words: Vec<OcrWord>,
        fonts: Vec<FontMatch>,
        /// The cuts (regular, bold, italic…) of the family the page is set in.
        variants: Vec<PathBuf>,
        /// The image the faces were matched against, at the scan's own dpi.
        image: Arc<GrayImage>,
        /// Factor from OCR pixels to that image's pixels.
        image_scale: f32,
    },
    OcrFailed { page: usize, gen: u64, error: String },
    FontSample { gen: u64, ranking: Vec<FontMatch> },
    Downloaded { family: String, path: PathBuf },
    DownloadFailed { family: String, error: String },
}

pub struct Worker {
    render_tx: Sender<Job>,
    ocr_tx: Sender<Job>,
    sample_tx: Sender<Job>,
    download_tx: Sender<usize>,
    rx: Receiver<Event>,
}

impl Worker {
    pub fn spawn(ctx: egui::Context) -> Self {
        let (tx, rx) = mpsc::channel::<Event>();
        let (render_tx, render_rx) = mpsc::channel::<Job>();
        let (ocr_tx, ocr_rx) = mpsc::channel::<Job>();
        let (download_tx, download_rx) = mpsc::channel::<usize>();
        let (sample_tx, sample_rx) = mpsc::channel::<Job>();

        let tx_render = tx.clone();
        let ctx_render = ctx.clone();
        thread::spawn(move || loop {
            let Ok(job) = render_rx.recv() else { return };
            let job = drain_latest(job, &render_rx);
            process(job, &tx_render, &ctx_render);
        });

        let tx_ocr = tx.clone();
        let ctx_ocr = ctx.clone();
        thread::spawn(move || loop {
            let Ok(job) = ocr_rx.recv() else { return };
            let job = drain_latest(job, &ocr_rx);
            process(job, &tx_ocr, &ctx_ocr);
        });

        // Font sampling runs on its own thread, never drained, so evidence from
        // every page is gathered even while the view OCRs the current one.
        let tx_sample = tx.clone();
        let ctx_sample = ctx.clone();
        thread::spawn(move || loop {
            let Ok(job) = sample_rx.recv() else { return };
            if let Job::SampleFonts { pdf, pages, lang, gen } = job {
                for (page, match_dpi) in pages {
                    let ranking = sample_page_fonts(&pdf, page, match_dpi, &lang);
                    if !ranking.is_empty() {
                        let _ = tx_sample.send(Event::FontSample { gen, ranking });
                        ctx_sample.request_repaint();
                    }
                }
            }
        });

        // Downloads get their own thread: unlike renders, a queued download must
        // never be dropped in favour of a newer one.
        thread::spawn(move || loop {
            let Ok(index) = download_rx.recv() else { return };
            let Some(entry) = crate::fontstore::CATALOG.get(index) else { continue };
            let event = match crate::fontstore::download(entry) {
                Ok(path) => Event::Downloaded { family: entry.family.to_string(), path },
                Err(e) => Event::DownloadFailed {
                    family: entry.family.to_string(),
                    error: format!("{e:#}"),
                },
            };
            let _ = tx.send(event);
            ctx.request_repaint();
        });

        Self { render_tx, ocr_tx, sample_tx, download_tx, rx }
    }

    pub fn render(&self, pdf: PathBuf, page: usize, dpi: u32, gen: u64) {
        let _ = self.render_tx.send(Job::Render { pdf, page, dpi, gen });
    }

    pub fn ocr(&self, pdf: PathBuf, page: usize, dpi: u32, match_dpi: u32, lang: String, gen: u64) {
        let _ = self.ocr_tx.send(Job::Ocr { pdf, page, dpi, match_dpi, lang, gen });
    }

    pub fn sample_fonts(&self, pdf: PathBuf, pages: Vec<(usize, u32)>, lang: String, gen: u64) {
        let _ = self.sample_tx.send(Job::SampleFonts { pdf, pages, lang, gen });
    }

    pub fn download_font(&self, catalog_index: usize) {
        let _ = self.download_tx.send(catalog_index);
    }

    pub fn poll(&self) -> Vec<Event> {
        let mut out = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            out.push(event);
        }
        out
    }
}

/// Keeps only the newest queued job: stale pages are never rendered.
fn drain_latest(current: Job, rx: &Receiver<Job>) -> Job {
    let mut job = current;
    while let Ok(next) = rx.try_recv() {
        job = next;
    }
    job
}

fn process(job: Job, tx: &Sender<Event>, ctx: &egui::Context) {
    let event = match job {
        Job::Render { pdf, page, dpi, gen } => {
            match crate::pdf::render::render_page_to_png_bytes(&pdf, page as u32 + 1, dpi) {
                Ok(bytes) => match decode_png(&bytes) {
                    Ok(image) => Event::Rendered { page, gen, image },
                    Err(error) => Event::RenderFailed { page, gen, error },
                },
                Err(e) => Event::RenderFailed { page, gen, error: e.to_string() },
            }
        }
        Job::Ocr { pdf, page, dpi, match_dpi, lang, gen } => run_ocr(&pdf, page, dpi, match_dpi, &lang, gen),
        Job::SampleFonts { .. } => return,
    };
    let _ = tx.send(event);
    ctx.request_repaint();
}

fn run_ocr(pdf: &std::path::Path, page: usize, dpi: u32, match_dpi: u32, lang: &str, gen: u64) -> Event {
    let bytes = match crate::pdf::render::render_page_to_png_bytes(pdf, page as u32 + 1, dpi) {
        Ok(b) => b,
        Err(e) => return Event::OcrFailed { page, gen, error: e.to_string() },
    };

    let dir = std::env::temp_dir().join("pdf_editor_ocr");
    let _ = std::fs::create_dir_all(&dir);
    let raw = dir.join(format!("raw_{page}.png"));
    let prepped = dir.join(format!("ocr_{page}.png"));

    if let Err(e) = std::fs::write(&raw, &bytes) {
        return Event::OcrFailed { page, gen, error: format!("Écriture temporaire : {e}") };
    }
    if crate::ocr::preprocess::preprocess_for_ocr(&raw, &prepped).is_err() {
        let _ = std::fs::copy(&raw, &prepped);
    }

    let mut engine = "Tesseract";
    let mut words = Vec::new();

    if crate::ocr::surya::is_available() {
        if let Ok(found) = crate::ocr::surya::ocr_image(&prepped) {
            if !found.is_empty() {
                engine = "Surya";
                words = found;
            }
        }
    }

    if words.is_empty() {
        if !crate::ocr::tesseract::is_available() {
            return Event::OcrFailed {
                page,
                gen,
                error: "Aucun moteur OCR (installez Tesseract ou surya-ocr)".into(),
            };
        }
        match crate::ocr::tesseract::ocr_word_positions(&prepped, lang) {
            Ok(found) => words = found,
            Err(e) => return Event::OcrFailed { page, gen, error: e.to_string() },
        }
    }

    // Identify the typeface, on a render at the scan's *own* resolution: going
    // finer than the paper was digitised at only upsamples blur, and the faces
    // then all look alike. The image travels with the result so a correction can
    // be matched against the exact word being edited, not the page average.
    let image_scale = match_dpi as f32 / dpi as f32;
    let match_bytes = if match_dpi == dpi {
        bytes
    } else {
        match crate::pdf::render::render_page_to_png_bytes(pdf, page as u32 + 1, match_dpi) {
            Ok(b) => b,
            Err(e) => return Event::OcrFailed { page, gen, error: e.to_string() },
        }
    };
    let gray = match image::load_from_memory(&match_bytes) {
        Ok(img) => img.to_luma8(),
        Err(e) => return Event::OcrFailed { page, gen, error: format!("Image illisible : {e}") },
    };

    let candidates = crate::fontstore::candidates();
    let scaled: Vec<OcrWord> = words.iter().map(|w| scale_word(w, image_scale)).collect();
    let fonts = crate::ocr::fontmatch::rank(&gray, &scaled, &candidates);

    let variants = match crate::ocr::fontmatch::page_family(&fonts) {
        Some(family) => crate::ocr::fontmatch::family_variants(&family, &candidates),
        None => Vec::new(),
    };

    Event::OcrDone {
        page,
        gen,
        engine: engine.to_string(),
        words,
        fonts,
        variants,
        image: Arc::new(gray),
        image_scale,
    }
}

/// The same word, expressed in the pixels of another render.
pub fn scale_word(word: &OcrWord, k: f32) -> OcrWord {
    OcrWord {
        text: word.text.clone(),
        bbox: (word.bbox.0 * k, word.bbox.1 * k, word.bbox.2 * k, word.bbox.3 * k),
        confidence: word.confidence,
    }
}

pub fn decode_png(bytes: &[u8]) -> Result<ColorImage, String> {
    let img = image::load_from_memory(bytes).map_err(|e| format!("Décodage image : {e}"))?;
    let rgba = img.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let pixels = rgba
        .chunks_exact(4)
        .map(|p| Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]))
        .collect();
    Ok(ColorImage { size, pixels })
}

/// OCRs one page and returns only its font ranking, for document-wide sampling.
fn sample_page_fonts(pdf: &std::path::Path, page: usize, match_dpi: u32, lang: &str) -> Vec<FontMatch> {
    let ocr_bytes = match crate::pdf::render::render_page_to_png_bytes(pdf, page as u32 + 1, 300) {
        Ok(b) => b,
        Err(_) => return Vec::new(),
    };
    let dir = std::env::temp_dir().join("pdf_editor_ocr");
    let _ = std::fs::create_dir_all(&dir);
    let raw = dir.join(format!("sample_{page}.png"));
    if std::fs::write(&raw, &ocr_bytes).is_err() {
        return Vec::new();
    }

    let words = match crate::ocr::tesseract::ocr_word_positions(&raw, lang) {
        Ok(w) if !w.is_empty() => w,
        _ => return Vec::new(),
    };

    let match_bytes = match crate::pdf::render::render_page_to_png_bytes(pdf, page as u32 + 1, match_dpi) {
        Ok(b) => b,
        Err(_) => return Vec::new(),
    };
    let gray = match image::load_from_memory(&match_bytes) {
        Ok(img) => img.to_luma8(),
        Err(_) => return Vec::new(),
    };
    let k = match_dpi as f32 / 300.0;
    let scaled: Vec<OcrWord> = words.iter().map(|w| scale_word(w, k)).collect();
    crate::ocr::fontmatch::rank(&gray, &scaled, &crate::fontstore::candidates())
}
