// =============================================================================
// PDF Lks Text Editor - Éditeur PDF avec OCR et édition de texte
// Créé par Leksa667 (https://github.com/Leksa667)
//
// Ce logiciel est en libre service. Vous pouvez l'utiliser, le modifier
// et le distribuer librement, à condition de créditer l'auteur original.
// Aucune garantie n'est fournie.
// =============================================================================

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

/// Open-licence faces that may legally be embedded in a PDF. The metric-
/// compatible ones stand in for the Microsoft core fonts a scan was likely set
/// in but which we are not allowed to redistribute.
pub struct CatalogEntry {
    pub family: &'static str,
    pub note: &'static str,
    pub file: &'static str,
    pub url: &'static str,
}

pub const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        family: "Arimo",
        note: "métriques d'Arial / Helvetica",
        file: "Arimo.ttf",
        url: "https://github.com/google/fonts/raw/main/ofl/arimo/Arimo%5Bwght%5D.ttf",
    },
    CatalogEntry {
        family: "Tinos",
        note: "métriques de Times New Roman",
        file: "Tinos-Regular.ttf",
        url: "https://github.com/google/fonts/raw/main/ofl/tinos/Tinos-Regular.ttf",
    },
    CatalogEntry {
        family: "Tinos Bold",
        note: "Times New Roman gras",
        file: "Tinos-Bold.ttf",
        url: "https://github.com/google/fonts/raw/main/ofl/tinos/Tinos-Bold.ttf",
    },
    CatalogEntry {
        family: "Tinos Italic",
        note: "Times New Roman italique",
        file: "Tinos-Italic.ttf",
        url: "https://github.com/google/fonts/raw/main/ofl/tinos/Tinos-Italic.ttf",
    },
    CatalogEntry {
        family: "Cousine",
        note: "métriques de Courier New",
        file: "Cousine-Regular.ttf",
        url: "https://github.com/google/fonts/raw/main/ofl/cousine/Cousine-Regular.ttf",
    },
    CatalogEntry {
        family: "Lato",
        note: "sans-serif humaniste",
        file: "Lato-Regular.ttf",
        url: "https://github.com/google/fonts/raw/main/ofl/lato/Lato-Regular.ttf",
    },
    CatalogEntry {
        family: "Lato Bold",
        note: "sans-serif humaniste gras",
        file: "Lato-Bold.ttf",
        url: "https://github.com/google/fonts/raw/main/ofl/lato/Lato-Bold.ttf",
    },
    CatalogEntry {
        family: "PT Sans",
        note: "sans-serif",
        file: "PT_Sans-Web-Regular.ttf",
        url: "https://github.com/google/fonts/raw/main/ofl/ptsans/PT_Sans-Web-Regular.ttf",
    },
];

pub fn cache_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("PdfEditor").join("fonts")
}

pub fn is_cached(entry: &CatalogEntry) -> bool {
    cache_dir().join(entry.file).is_file()
}

/// Downloads a catalog font into the per-user cache. Nothing is installed
/// system-wide and no administrator rights are needed: the file is used
/// straight from the cache and embedded into the PDF.
pub fn download(entry: &CatalogEntry) -> Result<PathBuf> {
    let dir = cache_dir();
    std::fs::create_dir_all(&dir).context("Création du cache de polices impossible")?;
    let target = dir.join(entry.file);
    if target.is_file() {
        return Ok(target);
    }

    let response = ureq::get(entry.url)
        .timeout(std::time::Duration::from_secs(30))
        .call()
        .with_context(|| format!("Téléchargement de {} impossible", entry.family))?;

    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut response.into_reader(), &mut bytes)
        .context("Lecture du flux réseau impossible")?;

    if bytes.len() < 1024 || &bytes[0..4] != b"\x00\x01\x00\x00" {
        return Err(anyhow!("Le fichier reçu n'est pas une police TrueType"));
    }

    // Write to a temporary file first: a half-downloaded font must never end up
    // in the cache under its final name.
    let temp = dir.join(format!("{}.part", entry.file));
    std::fs::write(&temp, &bytes).context("Écriture du cache impossible")?;
    std::fs::rename(&temp, &target).context("Finalisation du cache impossible")?;
    Ok(target)
}

/// Every TrueType face we may rasterize for matching and embed: the user's
/// installed fonts plus everything already downloaded into the cache.
pub fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut dirs = vec![cache_dir()];

    if let Some(win) = std::env::var_os("WINDIR") {
        dirs.push(Path::new(&win).join("Fonts"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(Path::new(&local).join("Microsoft").join("Windows").join("Fonts"));
    }

    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_ttf = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("ttf"))
                .unwrap_or(false);
            if is_ttf {
                out.push(path);
            }
        }
    }
    out
}
