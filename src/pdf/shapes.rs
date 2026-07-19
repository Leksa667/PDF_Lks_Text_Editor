// =============================================================================
// PDF Lks Text Editor - Éditeur PDF avec OCR et édition de texte
// Créé par Leksa667 (https://github.com/Leksa667)
//
// Ce logiciel est en libre service. Vous pouvez l'utiliser, le modifier
// et le distribuer librement, à condition de créditer l'auteur original.
// Aucune garantie n'est fournie.
// =============================================================================

use anyhow::Result;
use lopdf::content::{Content, Operation};
use lopdf::{Document, ObjectId};

#[derive(Clone, Debug)]
pub enum ShapeKind {
    Line { x1: f32, y1: f32, x2: f32, y2: f32 },
    Rect { x: f32, y: f32, w: f32, h: f32 },
    Circle { cx: f32, cy: f32, radius: f32 },
}

#[derive(Clone, Debug)]
pub struct Shape {
    pub kind: ShapeKind,
    pub color: [f32; 3],
    pub fill: bool,
    pub fill_color: [f32; 3],
    pub thickness: f32,
}

impl ShapeKind {
    pub fn bounds(&self) -> [f32; 4] {
        match self {
            ShapeKind::Line { x1, y1, x2, y2 } => {
                [x1.min(*x2), y1.min(*y2), x1.max(*x2), y1.max(*y2)]
            }
            ShapeKind::Rect { x, y, w, h } => {
                let (x2, y2) = if *w >= 0.0 { (x + w, y + h) } else { (x + w, y + h) };
                [x.min(x2), y.min(y2), x.max(x2), y.max(y2)]
            }
            ShapeKind::Circle { cx, cy, radius } => {
                [cx - radius, cy - radius, cx + radius, cy + radius]
            }
        }
    }

    pub fn hit_test(&self, px: f32, py: f32, threshold: f32) -> bool {
        match self {
            ShapeKind::Line { x1, y1, x2, y2 } => {
                let dx = x2 - x1;
                let dy = y2 - y1;
                let len2 = dx * dx + dy * dy;
                if len2 == 0.0 {
                    return ((px - x1).powi(2) + (py - y1).powi(2)).sqrt() <= threshold;
                }
                let t = ((px - x1) * dx + (py - y1) * dy) / len2;
                let t = t.clamp(0.0, 1.0);
                let cx = x1 + t * dx;
                let cy = y1 + t * dy;
                ((px - cx).powi(2) + (py - cy).powi(2)).sqrt() <= threshold
            }
            ShapeKind::Rect { x, y, w, h } => {
                let (x2, y2) = (*x + w, *y + h);
                let (xmin, xmax) = (x.min(x2), x.max(x2));
                let (ymin, ymax) = (y.min(y2), y.max(y2));
                px >= xmin - threshold && px <= xmax + threshold
                    && py >= ymin - threshold && py <= ymax + threshold
            }
            ShapeKind::Circle { cx, cy, radius } => {
                ((px - cx).powi(2) + (py - cy).powi(2)).sqrt() <= radius + threshold
            }
        }
    }

    pub fn move_by(&mut self, dx: f32, dy: f32) {
        match self {
            ShapeKind::Line { x1, y1, x2, y2 } => {
                *x1 += dx; *y1 += dy; *x2 += dx; *y2 += dy;
            }
            ShapeKind::Rect { x, y, .. } => {
                *x += dx; *y += dy;
            }
            ShapeKind::Circle { cx, cy, .. } => {
                *cx += dx; *cy += dy;
            }
        }
    }
}

const KAPPA: f32 = 0.5522847498;

fn circle_ops(cx: f32, cy: f32, radius: f32) -> Vec<Operation> {
    let k = KAPPA * radius;
    vec![
        Operation::new("m", vec![(cx + radius).into(), cy.into()]),
        Operation::new("c", vec![
            (cx + radius).into(), (cy + k).into(),
            (cx + k).into(), (cy + radius).into(),
            cx.into(), (cy + radius).into(),
        ]),
        Operation::new("c", vec![
            (cx - k).into(), (cy + radius).into(),
            (cx - radius).into(), (cy + k).into(),
            (cx - radius).into(), cy.into(),
        ]),
        Operation::new("c", vec![
            (cx - radius).into(), (cy - k).into(),
            (cx - k).into(), (cy - radius).into(),
            cx.into(), (cy - radius).into(),
        ]),
        Operation::new("c", vec![
            (cx + k).into(), (cy - radius).into(),
            (cx + radius).into(), (cy - k).into(),
            (cx + radius).into(), cy.into(),
        ]),
    ]
}

pub fn shape_pdf_ops(shape: &Shape) -> Vec<Operation> {
    let mut ops = Vec::new();

    match &shape.kind {
        ShapeKind::Line { x1, y1, x2, y2 } => {
            ops.push(Operation::new("q", vec![]));
            ops.push(Operation::new("RG", vec![
                shape.color[0].into(), shape.color[1].into(), shape.color[2].into(),
            ]));
            ops.push(Operation::new("w", vec![shape.thickness.into()]));
            ops.push(Operation::new("m", vec![(*x1).into(), (*y1).into()]));
            ops.push(Operation::new("l", vec![(*x2).into(), (*y2).into()]));
            ops.push(Operation::new("S", vec![]));
            ops.push(Operation::new("Q", vec![]));
        }
        ShapeKind::Rect { x, y, w, h } => {
            if shape.fill {
                ops.push(Operation::new("q", vec![]));
                ops.push(Operation::new("rg", vec![
                    shape.fill_color[0].into(), shape.fill_color[1].into(), shape.fill_color[2].into(),
                ]));
                ops.push(Operation::new("re", vec![(*x).into(), (*y).into(), (*w).into(), (*h).into()]));
                ops.push(Operation::new("f", vec![]));
                ops.push(Operation::new("Q", vec![]));
            }
            ops.push(Operation::new("q", vec![]));
            ops.push(Operation::new("RG", vec![
                shape.color[0].into(), shape.color[1].into(), shape.color[2].into(),
            ]));
            ops.push(Operation::new("w", vec![shape.thickness.into()]));
            ops.push(Operation::new("re", vec![(*x).into(), (*y).into(), (*w).into(), (*h).into()]));
            ops.push(Operation::new("S", vec![]));
            ops.push(Operation::new("Q", vec![]));
        }
        ShapeKind::Circle { cx, cy, radius } => {
            let path = circle_ops(*cx, *cy, *radius);
            if shape.fill {
                ops.push(Operation::new("q", vec![]));
                ops.push(Operation::new("rg", vec![
                    shape.fill_color[0].into(), shape.fill_color[1].into(), shape.fill_color[2].into(),
                ]));
                ops.extend(path.clone());
                ops.push(Operation::new("f", vec![]));
                ops.push(Operation::new("Q", vec![]));
            }
            ops.push(Operation::new("q", vec![]));
            ops.push(Operation::new("RG", vec![
                shape.color[0].into(), shape.color[1].into(), shape.color[2].into(),
            ]));
            ops.push(Operation::new("w", vec![shape.thickness.into()]));
            ops.extend(path);
            ops.push(Operation::new("S", vec![]));
            ops.push(Operation::new("Q", vec![]));
        }
    }

    ops
}

pub fn append_shapes_to_page(
    doc: &mut Document,
    page_id: ObjectId,
    shapes: &[Shape],
) -> Result<()> {
    if shapes.is_empty() {
        return Ok(());
    }
    let mut ops = Vec::new();
    for shape in shapes {
        ops.extend(shape_pdf_ops(shape));
    }
    crate::pdf::overlay::append_isolated(doc, page_id, Content { operations: ops })?;
    Ok(())
}
