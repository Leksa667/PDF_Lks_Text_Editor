// =============================================================================
// PDF Lks Text Editor - Éditeur PDF avec OCR et édition de texte
// Créé par Leksa667 (https://github.com/Leksa667)
//
// Ce logiciel est en libre service. Vous pouvez l'utiliser, le modifier
// et le distribuer librement, à condition de créditer l'auteur original.
// Aucune garantie n'est fournie.
// =============================================================================

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use pdf_editor::app::PdfEditorApp;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1500.0, 950.0])
            .with_min_inner_size([900.0, 600.0])
            .with_title("Éditeur PDF"),
        ..Default::default()
    };

    eframe::run_native(
        "Éditeur PDF",
        options,
        Box::new(|_cc| {
            let mut app = PdfEditorApp::new();
            if let Some(arg) = std::env::args().nth(1) {
                app.open_path(std::path::PathBuf::from(arg));
            }
            Box::new(app)
        }),
    )
}
