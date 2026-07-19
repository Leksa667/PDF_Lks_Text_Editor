// =============================================================================
// PDF Lks Text Editor - Éditeur PDF avec OCR et édition de texte
// Créé par Leksa667 (https://github.com/Leksa667)
//
// Ce logiciel est en libre service. Vous pouvez l'utiliser, le modifier
// et le distribuer librement, à condition de créditer l'auteur original.
// Aucune garantie n'est fournie.
// =============================================================================

use crate::pdf::render::no_window;
use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

static TESSERACT: OnceLock<Option<PathBuf>> = OnceLock::new();

#[derive(Clone, Debug)]
pub struct OcrWord {
    pub text: String,
    /// x, y, width, height in pixels of the OCR image
    pub bbox: (f32, f32, f32, f32),
    pub confidence: f32,
}

fn detect() -> Option<PathBuf> {
    for path in [
        r"C:\Program Files\Tesseract-OCR\tesseract.exe",
        r"C:\Program Files (x86)\Tesseract-OCR\tesseract.exe",
    ] {
        let p = Path::new(path);
        if p.exists() {
            return Some(p.to_path_buf());
        }
    }
    let mut cmd = Command::new("tesseract");
    if no_window(&mut cmd).arg("--version").output().is_ok() {
        return Some(PathBuf::from("tesseract"));
    }
    None
}

fn tess() -> Result<PathBuf> {
    TESSERACT
        .get_or_init(detect)
        .clone()
        .context("Tesseract introuvable : https://github.com/UB-Mannheim/tesseract/wiki")
}

pub fn is_available() -> bool {
    TESSERACT.get_or_init(detect).is_some()
}

fn run(image_path: &Path, lang: &str, tsv: bool) -> Result<String> {
    let exe = tess()?;
    let mut cmd = Command::new(&exe);
    no_window(&mut cmd)
        .arg(image_path)
        .arg("stdout")
        .arg("-l")
        .arg(lang)
        .arg("--psm")
        .arg("3");
    if tsv {
        cmd.arg("tsv");
    }
    let output = cmd.output().context("Impossible d'exécuter tesseract")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("Tesseract a échoué : {}", stderr.trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub fn ocr_word_positions(image_path: &Path, lang: &str) -> Result<Vec<OcrWord>> {
    let text = run(image_path, lang, true)?;
    let mut words = Vec::new();

    for line in text.lines().skip(1) {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 12 || cols[0] != "5" {
            continue;
        }
        let left: f32 = match cols[6].parse() { Ok(v) => v, Err(_) => continue };
        let top: f32 = match cols[7].parse() { Ok(v) => v, Err(_) => continue };
        let width: f32 = match cols[8].parse() { Ok(v) => v, Err(_) => continue };
        let height: f32 = match cols[9].parse() { Ok(v) => v, Err(_) => continue };
        let conf: f32 = match cols[10].parse() { Ok(v) => v, Err(_) => continue };
        let word = cols[11].trim().to_string();

        if word.is_empty() || conf < 40.0 {
            continue;
        }
        words.push(OcrWord {
            text: word,
            bbox: (left, top, width, height),
            confidence: conf,
        });
    }

    Ok(words)
}
