# PDF Lks Text Editor

Free PDF editor with OCR, font matching, text editing and vector drawing.

Built with Rust and [egui](https://github.com/emilk/egui).

## Features

- **Edit text** in existing PDFs — modify any text without breaking the layout
- **OCR** (Tesseract / Surya) — scanned documents become editable
- **Font matching** — automatically identifies the typeface of a scan for invisible corrections
- **Find & replace** — single page or entire document
- **Vector drawing** — lines, rectangles, circles with color and fill
- **Undo/Redo** (Ctrl+Z / Ctrl+Y)
- **Open-licensed fonts** — download from within the app (Arimo, Tinos, Lato…)
- **PDF export** — save as a new file

## Installation

### Prerequisites

- [Rust](https://www.rust-lang.org/) (2021 edition)
- A PDF renderer (one of these) :
  - [Ghostscript](https://ghostscript.com/) — recommended
  - mutool
  - pdftoppm (poppler)
- OCR (optional) :
  - [Tesseract](https://github.com/UB-Mannheim/tesseract/wiki) — `fra+eng` recommended
  - or [surya-ocr](https://github.com/VikParuchuri/surya) — `pip install surya-ocr`

### Build

```bash
cargo build --release
```

### Usage

```bash
cargo run --release
# or open a file directly :
cargo run --release -- "path/to/document.pdf"
```

Shortcuts :
| Key | Action |
|-----|--------|
| Ctrl+O | Open PDF |
| Ctrl+S | Save |
| Ctrl+Z | Undo |
| Ctrl+Y | Redo |
| Page Down/Up | Navigate |
| Ctrl+0 | Fit zoom |

## Architecture

```
src/
├── main.rs          # Entry point
├── lib.rs           # Module declarations
├── app.rs           # GUI (egui)
├── fontstore.rs     # Open font catalog
├── worker.rs        # Background threads : render, OCR, download
├── ocr/
│   ├── fontmatch.rs # Font matching algorithm
│   ├── tesseract.rs # Tesseract OCR interface
│   ├── surya.rs     # Surya OCR interface
│   └── preprocess.rs# Image preprocessing
└── pdf/
    ├── document.rs  # PDF document structure
    ├── edit.rs      # In-place text editing
    ├── embed.rs     # TrueType font embedding
    ├── font.rs      # PDF font encoding/decoding
    ├── overlay.rs   # Text painting over scans
    ├── render.rs    # PDF rendering via Ghostscript/mutool
    ├── shapes.rs    # Vector shapes
    ├── subset.rs    # TrueType font subsetting
    ├── text.rs      # Text run extraction
    └── ttf.rs       # TrueType parser
```

## License

This software is free to use, modify and distribute, provided that the original author is credited. No warranty is provided.

Created by [Leksa667](https://github.com/Leksa667).
