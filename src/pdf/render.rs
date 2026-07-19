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
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

static RENDERER: OnceLock<Option<PathBuf>> = OnceLock::new();
static COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn no_window(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

pub fn is_renderer_available() -> bool {
    find_renderer().is_some()
}

pub fn renderer_name() -> String {
    find_renderer()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_else(|| "aucun".into())
}

fn find_renderer() -> Option<PathBuf> {
    RENDERER.get_or_init(detect_renderer).clone()
}

fn detect_renderer() -> Option<PathBuf> {
    let candidates = [
        "gswin64c.exe",
        "gswin32c.exe",
        "gs.exe",
        "gs",
        "mutool.exe",
        "mutool",
        "pdftoppm.exe",
        "pdftoppm",
    ];

    for name in &candidates {
        if let Some(path) = which(name) {
            return Some(path);
        }
    }

    for dir in [r"C:\Program Files\gs", r"C:\Program Files (x86)\gs"] {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                for exe in ["gswin64c.exe", "gswin32c.exe"] {
                    let bin = entry.path().join("bin").join(exe);
                    if bin.exists() {
                        return Some(bin);
                    }
                }
            }
        }
    }
    None
}

fn which(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    for p in std::env::split_paths(&paths) {
        let candidate = p.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

pub fn render_page_to_png_bytes(pdf_path: &Path, page_num: u32, dpi: u32) -> Result<Vec<u8>> {
    let exe = find_renderer().context(
        "Aucun moteur de rendu PDF trouvé.\n\
         Installez Ghostscript : winget install ArtifexSoftware.GhostScript\n\
         (mutool ou pdftoppm sont aussi supportés)",
    )?;
    let exe_name = exe
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let tmp_dir = std::env::temp_dir().join("pdf_editor_render");
    std::fs::create_dir_all(&tmp_dir)?;

    let uid = COUNTER.fetch_add(1, Ordering::Relaxed);
    let output_path = tmp_dir.join(format!("p{page_num}_{uid}.png"));
    let inp = pdf_path.to_string_lossy().to_string();
    let out = output_path.to_string_lossy().to_string();

    let mut produced = output_path.clone();

    if exe_name.starts_with("gs") {
        let mut cmd = Command::new(&exe);
        no_window(&mut cmd)
            .args([
                "-dNOPAUSE",
                "-dBATCH",
                "-dSAFER",
                "-dQUIET",
                "-sDEVICE=png16m",
                "-dTextAlphaBits=4",
                "-dGraphicsAlphaBits=4",
            ])
            .arg(format!("-r{dpi}"))
            .arg(format!("-dFirstPage={page_num}"))
            .arg(format!("-dLastPage={page_num}"))
            .arg(format!("-sOutputFile={out}"))
            .arg(&inp);
        run(cmd, "Ghostscript")?;
    } else if exe_name == "mutool" {
        let mut cmd = Command::new(&exe);
        no_window(&mut cmd)
            .args(["draw", "-o", &out])
            .arg(format!("-r{dpi}"))
            .arg(&inp)
            .arg(page_num.to_string());
        run(cmd, "mutool")?;
    } else if exe_name == "pdftoppm" {
        let prefix = tmp_dir.join(format!("p{page_num}_{uid}"));
        let prefix_str = prefix.to_string_lossy().to_string();
        let mut cmd = Command::new(&exe);
        no_window(&mut cmd)
            .args([
                "-png",
                &format!("-r{dpi}"),
                &format!("-f{page_num}"),
                &format!("-l{page_num}"),
                "-singlefile",
            ])
            .arg(&inp)
            .arg(&prefix_str);
        run(cmd, "pdftoppm")?;
        produced = PathBuf::from(format!("{prefix_str}.png"));
    } else {
        return Err(anyhow!("Moteur de rendu non supporté : {exe_name}"));
    }

    let bytes = std::fs::read(&produced)
        .with_context(|| format!("Le moteur n'a produit aucune image ({})", produced.display()))?;
    let _ = std::fs::remove_file(&produced);
    Ok(bytes)
}

fn run(mut cmd: Command, name: &str) -> Result<()> {
    let output = cmd
        .output()
        .with_context(|| format!("Échec de l'exécution de {name}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("{name} a échoué : {}", err.trim()));
    }
    Ok(())
}
