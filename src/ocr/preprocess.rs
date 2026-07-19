// =============================================================================
// PDF Lks Text Editor - Éditeur PDF avec OCR et édition de texte
// Créé par Leksa667 (https://github.com/Leksa667)
//
// Ce logiciel est en libre service. Vous pouvez l'utiliser, le modifier
// et le distribuer librement, à condition de créditer l'auteur original.
// Aucune garantie n'est fournie.
// =============================================================================

use anyhow::Result;
use image::{DynamicImage, GenericImageView, ImageBuffer, Luma};
use std::path::Path;

fn luminance(r: u8, g: u8, b: u8) -> u8 {
    (0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32) as u8
}

fn autocontrast(pixels: &mut [u8]) {
    let mut hist = [0u32; 256];
    for &p in pixels.iter() {
        hist[p as usize] = hist[p as usize].saturating_add(1);
    }

    let total = pixels.len() as u32;
    let clip = total / 100;

    let mut cum = 0u32;
    let mut lo = 0usize;
    for (i, &h) in hist.iter().enumerate() {
        cum += h;
        if cum > clip {
            lo = i;
            break;
        }
    }

    cum = 0;
    let mut hi = 255usize;
    for (i, &h) in hist.iter().enumerate().rev() {
        cum += h;
        if cum > clip {
            hi = i;
            break;
        }
    }

    if hi <= lo { return; }

    let scale = 255.0 / (hi - lo) as f32;
    for p in pixels.iter_mut() {
        let v = (*p as f32 - lo as f32) * scale;
        *p = v.clamp(0.0, 255.0) as u8;
    }
}

fn unsharp_mask(pixels: &mut [u8], w: usize, h: usize, strength: f32) {
    let src = pixels.to_vec();
    // Simple 3x3 blur
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let idx = y * w + x;
            let blurred = (src[(y - 1) * w + (x - 1)] as u32
                + src[(y - 1) * w + x] as u32
                + src[(y - 1) * w + (x + 1)] as u32
                + src[y * w + (x - 1)] as u32
                + src[y * w + x] as u32
                + src[y * w + (x + 1)] as u32
                + src[(y + 1) * w + (x - 1)] as u32
                + src[(y + 1) * w + x] as u32
                + src[(y + 1) * w + (x + 1)] as u32) / 9;

            let diff = src[idx] as i32 - blurred as i32;
            let val = (src[idx] as f32 + diff as f32 * strength).round() as i32;
            pixels[idx] = val.clamp(0, 255) as u8;
        }
    }
}

pub fn preprocess_for_ocr(input_path: &Path, output_path: &Path) -> Result<()> {
    let img = image::open(input_path)?;
    let (w, h) = img.dimensions();
    let gray: ImageBuffer<Luma<u8>, Vec<u8>> = match img {
        DynamicImage::ImageLuma8(g) => g,
        DynamicImage::ImageRgb8(rgb) => {
            let mut buf = vec![0u8; (w * h) as usize];
            for (x, y, p) in rgb.enumerate_pixels() {
                buf[(y * w + x) as usize] = luminance(p[0], p[1], p[2]);
            }
            ImageBuffer::from_raw(w, h, buf).unwrap()
        }
        DynamicImage::ImageRgba8(rgba) => {
            let mut buf = vec![0u8; (w * h) as usize];
            for (x, y, p) in rgba.enumerate_pixels() {
                buf[(y * w + x) as usize] = luminance(p[0], p[1], p[2]);
            }
            ImageBuffer::from_raw(w, h, buf).unwrap()
        }
        other => other.to_luma8(),
    };

    let mut pixels = gray.into_raw();

    autocontrast(&mut pixels);
    unsharp_mask(&mut pixels, w as usize, h as usize, 0.8);

    let result = DynamicImage::ImageLuma8(
        ImageBuffer::from_raw(w, h, pixels).unwrap()
    );
    result.save(output_path)?;
    Ok(())
}
