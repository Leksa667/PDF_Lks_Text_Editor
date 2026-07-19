// =============================================================================
// PDF Lks Text Editor - Éditeur PDF avec OCR et édition de texte
// Créé par Leksa667 (https://github.com/Leksa667)
//
// Ce logiciel est en libre service. Vous pouvez l'utiliser, le modifier
// et le distribuer librement, à condition de créditer l'auteur original.
// Aucune garantie n'est fournie.
// =============================================================================

use crate::ocr::tesseract::OcrWord;
use crate::pdf::render::no_window;
use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

const MIN_CONFIDENCE: f32 = 0.3;
static PYTHON: OnceLock<Option<PathBuf>> = OnceLock::new();

const WRAPPER_SCRIPT: &str = r#"
import json, sys
from PIL import Image
from surya.detection import DetectionPredictor
from surya.recognition import RecognitionPredictor

img = Image.open(sys.argv[1])
det = DetectionPredictor()
rec = RecognitionPredictor()
results = rec([img], det_predictor=det)

out = []
for page in results:
    out.append({"text_lines": [
        {"text": tl.text, "bbox": tl.bbox, "confidence": tl.confidence}
        for tl in page.text_lines
    ]})
print("@@JSON@@" + json.dumps(out, ensure_ascii=False))
"#;

/// Detects a Python interpreter that actually has surya installed.
fn detect() -> Option<PathBuf> {
    for name in ["python", "python3", "py"] {
        let mut cmd = Command::new(name);
        let ok = no_window(&mut cmd)
            .args(["-c", "import surya, PIL"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if ok {
            return Some(PathBuf::from(name));
        }
    }
    None
}

pub fn is_available() -> bool {
    PYTHON.get_or_init(detect).is_some()
}

pub fn ocr_image(image_path: &Path) -> Result<Vec<OcrWord>> {
    let py = PYTHON
        .get_or_init(detect)
        .clone()
        .context("Surya non installé (pip install surya-ocr)")?;

    let mut cmd = Command::new(&py);
    let output = no_window(&mut cmd)
        .args(["-c", WRAPPER_SCRIPT])
        .arg(image_path)
        .output()
        .context("Échec d'exécution de Surya OCR")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let last = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("");
        return Err(anyhow!("Surya a échoué : {}", last.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json_str = stdout
        .split("@@JSON@@")
        .nth(1)
        .ok_or_else(|| anyhow!("Réponse Surya invalide"))?
        .trim();

    let pages: Vec<serde_json::Value> =
        serde_json::from_str(json_str).context("JSON Surya illisible")?;

    let mut words = Vec::new();
    for page in &pages {
        let Some(lines) = page["text_lines"].as_array() else { continue };
        for line in lines {
            let text = line["text"].as_str().unwrap_or("").trim().to_string();
            if text.is_empty() {
                continue;
            }
            let Some(arr) = line["bbox"].as_array() else { continue };
            if arr.len() < 4 {
                continue;
            }
            let v: Vec<f32> = arr.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
            let confidence = line["confidence"].as_f64().unwrap_or(1.0) as f32;
            if confidence < MIN_CONFIDENCE {
                continue;
            }
            words.push(OcrWord {
                text,
                bbox: (v[0], v[1], v[2] - v[0], v[3] - v[1]),
                confidence,
            });
        }
    }

    Ok(words)
}
