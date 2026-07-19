# PDF Lks Text Editor

Éditeur PDF libre avec OCR, reconnaissance de polices, édition de texte et dessin vectoriel.

Développé en Rust avec [egui](https://github.com/emilk/egui).

## Fonctionnalités

- **Édition de texte** dans les PDF existants — modifiez n'importe quel texte sans perdre la mise en page
- **OCR** (Tesseract / Surya) — les documents scannés deviennent éditables
- **Reconnaissance de polices** — identification automatique de la police d'un scan pour des corrections invisibles
- **Rechercher & remplacer** — sur une page ou tout le document
- **Dessin vectoriel** — lignes, rectangles, cercles, avec couleur et remplissage
- **Annuler/Rétablir** (Ctrl+Z / Ctrl+Y)
- **Polices libres intégrées** — téléchargement depuis l'application (Arimo, Tinos, Lato…)
- **Export PDF** — sauvegarde dans un nouveau fichier

## Installation

### Prérequis

- [Rust](https://www.rust-lang.org/) (édition 2021)
- Un moteur de rendu PDF (un des trois) :
  - [Ghostscript](https://ghostscript.com/) — recommandé
  - mutool
  - pdftoppm (poppler)
- OCR (optionnel) :
  - [Tesseract](https://github.com/UB-Mannheim/tesseract/wiki) — `fra+eng` recommandé
  - ou [surya-ocr](https://github.com/VikParuchuri/surya) — `pip install surya-ocr`

### Compilation

```bash
cargo build --release
```

### Utilisation

```bash
cargo run --release
# ou ouvrir un fichier directement :
cargo run --release -- "chemin/vers/document.pdf"
```

Raccourcis :
| Touche | Action |
|--------|--------|
| Ctrl+O | Ouvrir un PDF |
| Ctrl+S | Enregistrer |
| Ctrl+Z | Annuler |
| Ctrl+Y | Rétablir |
| Page suivante/précédente | Navigation |
| Ctrl+0 | Ajuster le zoom |

## Architecture

```
src/
├── main.rs          # Point d'entrée
├── lib.rs           # Modules
├── app.rs           # Interface utilisateur (egui)
├── fontstore.rs     # Catalogue de polices libres
├── worker.rs        # Threads : rendu, OCR, téléchargement
├── ocr/
│   ├── fontmatch.rs # Algorithme de correspondance de polices
│   ├── tesseract.rs # Interface Tesseract OCR
│   ├── surya.rs     # Interface Surya OCR
│   └── preprocess.rs# Prétraitement d'image
└── pdf/
    ├── document.rs  # Structure du document PDF
    ├── edit.rs      # Édition de texte in-place
    ├── embed.rs     # Intégration de polices TrueType
    ├── font.rs      # Encodage/décodage des polices PDF
    ├── overlay.rs   # Peinture de texte sur scans
    ├── render.rs    # Rendu PDF via Ghostscript/mutool
    ├── shapes.rs    # Formes vectorielles
    ├── subset.rs    # Sous-ensemble de polices TrueType
    ├── text.rs      # Extraction de blocs de texte
    └── ttf.rs       # Parseur TrueType
```

## Licence

Ce logiciel est en libre service. Vous pouvez l'utiliser, le modifier et le distribuer librement, à condition de créditer l'auteur original. Aucune garantie n'est fournie.

Créé par [Leksa667](https://github.com/Leksa667).
