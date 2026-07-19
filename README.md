# PDF Lks Text Editor

Free PDF editor with OCR, font matching, text editing and vector drawing.

Built with Rust and [egui](https://github.com/emilk/egui).

Created by [Leksa667](https://github.com/Leksa667).

## Features

- **Edit text** in existing PDFs — modify any text run (Tj, TJ, ', ") while preserving font, size and position
- **OCR** (Tesseract / Surya) — scanned documents become editable: mask the scan ink and repaint corrected text using embedded fonts
- **Font matching** — automatically identifies the typeface of a scanned page by comparing ink coverage maps against every installed font, using segmented ZNCC correlation, column/row profiles, ink density and a Bayesian usage prior
- **Per-word font selection** — distinguishes regular vs bold/italic within the same family at edit time
- **Find & replace** — single page or entire document, with glyph-availability checks
- **Vector drawing** — lines, rectangles, circles with configurable stroke color, fill color and thickness. Bezier circle approximation (Kappa = 0.55228)
- **Shape selection and movement** — click-to-select, drag-to-move, delete shapes
- **Undo/Redo** (Ctrl+Z / Ctrl+Y) — 100-level undo stack storing raw page content snapshots
- **Open-licensed font download** — built-in catalog (Arimo, Tinos, Lato, PT Sans, Cousine) downloaded into `%LOCALAPPDATA%\PdfEditor\fonts` with atomic file writes via `.part` temp files
- **Subset font embedding** — embeds only the needed glyphs (printable ASCII + Latin-1 + French typographic marks + edit characters) as CIDFontType2/Identity-H with /ToUnicode CMap, keeping file sizes under 25% of the original font
- **Composite glyph support** — recursive component tracking (depth ≤ 8) for proper subsetting
- **TrueType parser** — reads head, maxp, hhea, hmtx, OS/2, post, name and cmap tables
- **Rotated page support** — /Rotate 0/90/180/270: text axes automatically adjusted, coordinates converted between visual and PDF user space
- **Unbalanced graphics state protection** — `append_isolated()` wraps existing content in `q...Q` so scanner artifacts (unclosed cm, clipping paths, stale colors) never affect painted corrections
- **Concurrent rendering** — 4 background threads: render (newest-wins drain), OCR (newest-wins drain), font sampling (sequential, no drain), font download (sequential, no drain)

## Installation

### Prerequisites

- [Rust](https://www.rust-lang.org/) (2021 edition)
- A PDF renderer (one of these, auto-detected in this order):
  - [Ghostscript](https://ghostscript.com/) — recommended (`gswin64c`, `gswin32c` or `gs`)
  - mutool — `mutool draw`
  - pdftoppm (poppler) — `pdftoppm -png -singlefile`
- OCR (optional, auto-detected):
  - [Tesseract](https://github.com/UB-Mannheim/tesseract/wiki) v5+ — language packs: `fra+eng` recommended
  - [surya-ocr](https://github.com/VikParuchuri/surya) — `pip install surya-ocr` (tried first, falls back to Tesseract)

### Build

```bash
cargo build --release
```

### Usage

```bash
cargo run --release
# or open a file directly:
cargo run --release -- "path/to/document.pdf"
```

### Shortcuts

| Key | Action |
|-----|--------|
| Ctrl+O | Open PDF |
| Ctrl+S | Save |
| Ctrl+Z | Undo |
| Ctrl+Y | Redo |
| Page Down/Up | Navigate pages |
| Ctrl+0 | Fit zoom |
| Escape | Cancel editing |
| Delete | Remove selected shape |
| Mouse wheel | Vertical scroll |
| Ctrl+wheel | Zoom in/out |
| Drag | Pan page |
| Click text run | Edit text |
| Click OCR word | Edit scanned word |
| Click shape | Select shape |
| Drag shape | Move shape |

## Architecture

```
src/
├── main.rs              # Entry point — eframe window 1500×950, CLI arg support
├── lib.rs               # Module declarations
├── app.rs               # GUI and application logic (~2000 lines)
│   ├── PdfEditorApp     # 53 fields: document state, OCR, font matching, shapes
│   ├── ui()             # egui panels (top bar, side panel, page view)
│   ├── commit_ocr_edit()# Scan word rewriting: fit-to-box, reflow, follower absorption
│   ├── font_for_word()  # Per-word font cut selection (regular vs bold/italic)
│   ├── record_font_votes()# Document-wide font aggregation (max-score, not average)
│   ├── replace_all()    # Search & replace across single page or whole document
│   └── page_view()      # Zoom (0.05×–16×), pan, click detection, shape drawing
├── fontstore.rs         # Open font catalog + download manager
│   ├── CatalogEntry     # 8 fonts: Arimo, Tinos (×3), Cousine, Lato (×2), PT Sans
│   ├── download()       # HTTP fetch via ureq + TTF validation + atomic write
│   └── candidates()     # All .ttf from cache, WINDIR\Fonts, LOCALAPPDATA\Fonts
├── worker.rs            # Background thread manager
│   ├── Worker           # 4 mpsc channels + event rx
│   ├── render thread    # Ghostscript/mutool/pdftoppm → ColorImage
│   ├── ocr thread       # OCR pipeline + font matching + image capture
│   ├── sample thread    # Document-wide font sampling (up to 8 pages)
│   ├── download thread  # Sequential font downloads
│   └── drain_latest()   # Newest-wins job draining for render/OCR
├── ocr/
│   ├── mod.rs
│   ├── tesseract.rs     # Tesseract CLI wrapper
│   │   ├── OcrWord      # {text, bbox: (x,y,w,h), confidence}
│   │   ├── ocr_word_positions()# TSV parsing, confidence filter ≥ 40, word-only
│   │   └── detect()     # PATH scan + Program Files lookup + OnceLock cache
│   ├── surya.rs         # Surya OCR Python wrapper
│   │   ├── Inline Python script (DetectionPredictor + RecognitionPredictor)
│   │   ├── JSON parsing with @@JSON@@ delimiter
│   │   └── Confidence filter ≥ 0.3
│   └── preprocess.rs    # Image preprocessing pipeline
│       ├── Grayscale conversion (luminance: 0.299R + 0.587G + 0.114B)
│       ├── Autocontrast (1% clip on both ends)
│       └── Unsharp mask (3×3, strength 0.8)
└── pdf/
    ├── mod.rs
    ├── document.rs      # PDF document wrapper
    │   ├── PdfDocument  # lopdf::Document + page_ids + undo/redo stacks
    │   ├── geometry()   # MediaBox + Rotate extraction (Parent chain up to 32)
    │   ├── is_scanned() # Image present + text < 200 chars = scan
    │   ├── scan_dpi()   # Native DPI from largest page image
    │   └── undo/redo    # 100-level, raw page content snapshots
    ├── edit.rs          # In-place text editing
    │   ├── set_run_text()# Replace Tj/TJ/'/" operand, preserve font + size + pos
    │   └── move_run_by()# Adjust last Tm/Td/TD or insert new Tm
    ├── embed.rs         # TrueType font embedding
    │   ├── embed_font() # Subset → CIDFontType2/Identity-H with /ToUnicode
    │   ├── build_to_unicode()# CID→Unicode CMap generation (100 entries per chunk)
    │   ├── build_widths()# /W array with consecutive-run grouping
    │   └── encode_for_resource()# Encode using the PDF's CID mapping
    ├── font.rs          # PDF font encoding engine
    │   ├── FontEncoder  # Bidirectional code↔char mapping
    │   ├── build()      # Priority: /ToUnicode > embedded cmap > /Encoding
    │   ├── encode()     # Greedy longest-match, typographic substitutions
    │   ├── parse_to_unicode()# Full CMap parser (bfchar, bfrange, array form)
    │   └── parse_truetype_cmap()# Formats 0/4/6/12, symbol font 0xF000 handling
    ├── overlay.rs       # Scan text repainting
    │   ├── paint_line() # Mask + text placement with h_scale (70–112%)
    │   ├── fit_to_box() # Font size from InkBox (real ink extent, not metrics)
    │   ├── ink_box()    # fontdue rasterization at 100em to find exact ink bounds
    │   ├── visual_to_user()# Visual coords → PDF user space (rotation-aware)
    │   ├── text_axes()  # Transformation matrix for upright text on rotated pages
    │   └── append_isolated()# q/Q wrapping to isolate from scanner graphics state
    ├── render.rs        # PDF → PNG rendering
    │   ├── Auto-detection: Ghostscript → mutool → pdftoppm
    │   ├── Ghostscript: png16m, TextAlphaBits=4, GraphicsAlphaBits=4
    │   ├── Temp file management with atomic counter
    │   └── no_window()  # CREATE_NO_WINDOW flag on Windows
    ├── shapes.rs        # Vector shape operations
    │   ├── ShapeKind    # Line, Rect, Circle
    │   ├── shape_pdf_ops()# PDF operators with fill/stroke
    │   ├── hit_test()   # Point-in-shape with threshold (line projection)
    │   └── Circle: 4-cubic-Bezier approximation (Kappa = 0.5522847498)
    ├── subset.rs        # TrueType font subsetting
    │   ├── subset()     # Keep only needed glyphs + base_charset + components
    │   ├── component_fields()# Composite glyph parsing (flags: ARG_1_2_ARE_WORDS, MORE_COMPONENTS…)
    │   ├── add_components()# Recursive component gathering (max depth 8)
    │   ├── base_charset()# ASCII 0x20–0x7F + Latin-1 0xA0–0xFF + French specials
    │   └── Tables: glyf, head, hhea, hmtx, loca, maxp (no cmap — Identity-H)
    ├── text.rs          # Text run extraction
    │   ├── extract_runs()# Content stream interpreter: BT/ET, Tm/Td/TD/T*, Tf/Tc/Tw/Tz/Ts
    │   ├── Mat          # 2×3 affine matrix (row-vector convention)
    │   ├── FontMetrics  # Widths, default_width, ascent/descent per font
    │   ├── parse_cid_widths()# CID /W array parser (sequential + range + list)
    │   └── Coordinate conversion: pdf↔image (rotation-aware, 4 orientations)
    └── ttf.rs           # TrueType font parser
        ├── TtfFont      # Load .ttf from disk, parse all required tables
        ├── head: unitsPerEm, bbox, macStyle
        ├── hhea: ascent, descent, numHMetrics
        ├── hmtx: advance widths + left side bearings (raw + scaled)
        ├── OS/2: usWeightClass (bold ≥ 600), fsSelection, typo metrics, capHeight
        ├── post: italicAngle (16.16 fixed-point)
        ├── name: Family name (Windows Unicode platform preferred)
        ├── gid() / advance() / text_width() / pdf_name() / style_name()
        └── Rejects CFF/OpenType (no glyf table) and fonts without cmap

## Font Matching Algorithm

The font matcher (`ocr::fontmatch`) identifies the typeface of a scanned page in two passes:

### Pass 1 — Quick elimination
- Take the highest-confidence OCR word (min 4 chars, min 12×12 px)
- Rasterize every candidate font at the scan's native pixel height
- Compute a column-profile correlation with band-local alignment (4 bands, shift ±3)
- Keep the top 64 candidates

### Pass 2 — Full comparison
For each shortlisted candidate, against up to 8 words:
1. **Shape correlation** (`segmented_shape`): Zero-mean normalized cross-correlation of ink maps, divided into 4 column bands with independent (dx, dy) alignment (±3 px, ±2 px). Weight: ×1.2
2. **Column profile** (`profile_correlation`): ZNCC of column-density profiles, per band. Weight: ×0.8
3. **Row profile** (`row_correlation`): ZNCC of row-density profiles, ±2 shift. Weight: ×0.5
4. **Width ratio** (`×4`): Heavily weighted — wrong typeface produces wrong word length
5. **Ink density**: Scanned ink at 45% threshold vs rendered ink at 70% threshold, compensating for Otsu thinning vs crisp raster thickening. 0.5 + 0.5 × similarity
6. **Bayesian prior**: Document fonts (Arial, Times, etc.) score 1.0; script/symbol fonts score 0.72; unknown fonts score 0.88

Score = shape¹·² × profile⁰·⁸ × rows⁰·⁵ × width⁴ × (0.5 + 0.5×density) × prior

The final score is a trimmed mean (keep top ⅔ of words per candidate).

### Key insights
- Ink coverage maps (0.0 = paper, 1.0 = full ink) preserve stroke detail that binary thresholding destroys at scanner resolution (≈30 px word height)
- `binarized_to_ink()` normalizes ink fraction between scan and render — the scanner thins strokes (Otsu on blurred paper), while crisp rasterization fattens them. Matching at equal ink removes this bias
- Band-local alignment absorbs letter drift inside the word (accumulated registration error between print and scan) that a global shift cannot correct
- The prior breaks ties at low resolution where Times New Roman and Mongolian Baiti score identically

## Tests

20 tests across 4 files:

| File | Tests | Coverage |
|------|-------|----------|
| `tests/engine.rs` | 6 | Text extraction, in-place editing, CJK rejection, pixel-level render verification, overlay cycle, mask coverage |
| `tests/scan_font.rs` | 10 | Font recognition (Arial/Times/Courier), embedding+editing, font download, pixel-accurate placement (including x-height-only words), subset size (<10%), rotated pages (0/90/180/270), unbalanced scanner matrix, 9-font discrimination, reflow collision avoidance |
| `tests/fonts.rs` | 6 | Custom /Encoding with /Differences, CIDFontType2 + /ToUnicode, typographic substitutions, real cmap parsing, embedded cmap fallback |
| `tests/app_flow.rs` | 4 | Full app flow (open→edit→save→verify on disk), newline sanitization, impossible edit reporting, scan vs text page detection |

```bash
cargo test
```

## Examples

| File | Purpose |
|------|---------|
| `examples/apply.rs` | Batch edit a scanned PDF: replace dates and text across all pages using overlay with embedded Arial |
| `examples/fontdiag.rs` | Diagnostic: simulate a scan, compare font similarity scores, render ASCII ink maps |
| `examples/fontbench.rs` | Benchmark: test every system font for self-recognition across 6 words, report top-1/top-3/visual equivalence rates |

## Dependencies

### Rust crates
| Crate | Version | Usage |
|-------|---------|-------|
| `lopdf` | 0.44 | PDF read/write, content stream parsing, font dictionaries |
| `egui` | 0.27 | Immediate-mode GUI (panels, textures, input handling) |
| `eframe` | 0.27 | Application framework (window, event loop) |
| `image` | 0.24 | PNG decode, grayscale conversion, pixel manipulation |
| `serde_json` | 1 | Surya OCR JSON response parsing |
| `anyhow` | 1 | Contextual error handling |
| `rfd` | 0.12 | Native file-open/save dialogs |
| `fontdue` | 0.9 | Font rasterization for coverage map generation |
| `ureq` | 2 | HTTP client for font downloads |

### System dependencies (external)
- Ghostscript, mutool or pdftoppm — PDF page rendering to PNG
- Tesseract OCR — word-level OCR with positional TSV output
- Python + surya-ocr — optional, higher priority than Tesseract when available

## License

This software is free to use, modify and distribute, provided that the original author is credited. No warranty is provided.

Created by [Leksa667](https://github.com/Leksa667).
