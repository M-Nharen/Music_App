//! The circle of fifths: major keys round the outside, their relative
//! minors inside. Lights the current key and its two neighbours (the
//! "family" of chords that sound natural together) and marks the chord
//! being played.

use crate::learn::analysis::Family;
use crate::theme::Palette;
use crate::theory::pc_name;
use egui::{Align2, Color32, FontId, Mesh, Pos2, Sense, Shape, Stroke, Ui, vec2};
use std::f32::consts::{PI, TAU};

/// Sharps (+) or flats (−) in the major key at position `i` (C = 0).
fn signature(i: usize) -> i32 {
    if i <= 6 { i as i32 } else { i as i32 - 12 }
}

fn major_name(pc: i32) -> String {
    match pc.rem_euclid(12) {
        6 => "F♯/G♭".into(),
        pc => pc_name(pc, matches!(pc, 1 | 3 | 5 | 8 | 10)).replace('b', "♭"),
    }
}

fn minor_name(pc: i32) -> String {
    let name = match pc.rem_euclid(12) {
        3 => "D♯/E♭".to_string(),
        pc => pc_name(pc, matches!(pc, 10 | 5 | 0 | 7 | 2))
            .replace('b', "♭")
            .replace('#', "♯"),
    };
    format!("{name}m")
}

/// Points along an annular sector, split into convex quads.
fn sector(center: Pos2, r0: f32, r1: f32, a0: f32, a1: f32) -> Vec<[Pos2; 4]> {
    let steps = 6;
    let at = |r: f32, a: f32| center + vec2(a.cos(), a.sin()) * r;
    (0..steps)
        .map(|s| {
            let x0 = a0 + (a1 - a0) * s as f32 / steps as f32;
            let x1 = a0 + (a1 - a0) * (s + 1) as f32 / steps as f32;
            [at(r0, x0), at(r1, x0), at(r1, x1), at(r0, x1)]
        })
        .collect()
}

/// `key_pc`: the key's tonic. `minor_key`: the key's home chord is minor.
/// `chord`: (root pitch class, family) of the chord sounding, if any.
pub fn circle_of_fifths(
    ui: &mut Ui,
    p: &Palette,
    size: f32,
    key_pc: i32,
    minor_key: bool,
    chord: Option<(i32, Family)>,
) {
    let size = size.max(160.0);
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let painter = ui.painter_at(rect);
    let c = rect.center();
    let r_out = size * 0.49;
    let r_mid = size * 0.33;
    let r_in = size * 0.19;
    // The major key whose family is lit (a minor key lights its relative major).
    let major_pc = if minor_key { key_pc + 3 } else { key_pc }.rem_euclid(12);
    let pos_of = |pc: i32| ((pc.rem_euclid(12) * 7) % 12) as usize;
    let key_i = pos_of(major_pc);
    let chord_i = chord.map(|(root, fam)| match fam {
        Family::Minor | Family::Dim => (pos_of(root + 3), false),
        _ => (pos_of(root), true),
    });

    for i in 0..12usize {
        let a_mid = -PI / 2.0 + i as f32 * TAU / 12.0;
        let (a0, a1) = (a_mid - TAU / 24.0 + 0.01, a_mid + TAU / 24.0 - 0.01);
        let dist = (i as i32 - key_i as i32)
            .rem_euclid(12)
            .min((key_i as i32 - i as i32).rem_euclid(12));
        let (outer_fill, inner_fill, fg) = match dist {
            0 => (
                p.primary_container,
                p.primary_container,
                p.on_primary_container,
            ),
            1 => (
                p.secondary_container,
                p.secondary_container,
                p.on_secondary_container,
            ),
            _ => (
                p.surface_container_high,
                p.surface_container,
                p.on_surface_variant,
            ),
        };
        let is_home_outer = i == key_i && !minor_key;
        let is_home_inner = i == key_i && minor_key;
        for (r0, r1, fill, outer) in [
            (r_mid, r_out, outer_fill, true),
            (r_in, r_mid - 2.0, inner_fill, false),
        ] {
            let mut fill = fill;
            if chord_i == Some((i, outer)) {
                fill = p.tertiary;
            }
            // One mesh per sector, so there are no seams between pieces.
            let mut mesh = Mesh::default();
            for q in sector(c, r0, r1, a0, a1) {
                let i = mesh.vertices.len() as u32;
                for pt in q {
                    mesh.colored_vertex(pt, fill);
                }
                mesh.add_triangle(i, i + 1, i + 2);
                mesh.add_triangle(i, i + 2, i + 3);
            }
            painter.add(Shape::mesh(mesh));
            let home = if outer { is_home_outer } else { is_home_inner };
            if home {
                let pts: Vec<Pos2> = (0..=12)
                    .map(|s| {
                        let a = a0 + (a1 - a0) * s as f32 / 12.0;
                        c + vec2(a.cos(), a.sin()) * (r1 - 1.5)
                    })
                    .collect();
                painter.add(Shape::line(pts, Stroke::new(3.0, p.primary)));
            }
        }
        let text_col = |on_chord: bool| if on_chord { p.surface } else { fg };
        let dir = vec2(a_mid.cos(), a_mid.sin());
        let pc_major = (i as i32 * 7).rem_euclid(12);
        painter.text(
            c + dir * (r_mid + r_out) / 2.0 - vec2(0.0, 4.0),
            Align2::CENTER_CENTER,
            major_name(pc_major),
            FontId::proportional((size * 0.055).clamp(10.0, 15.0)),
            text_col(chord_i == Some((i, true))),
        );
        let sig = signature(i);
        let sig_text = match sig {
            0 => String::new(),
            s if s > 0 => format!("{s}♯"),
            s => format!("{}♭", -s),
        };
        if !sig_text.is_empty() && i != 6 {
            painter.text(
                c + dir * (r_mid + r_out) / 2.0 + vec2(0.0, 8.0),
                Align2::CENTER_CENTER,
                sig_text,
                FontId::proportional((size * 0.036).clamp(8.0, 10.0)),
                text_col(chord_i == Some((i, true))).gamma_multiply(0.75),
            );
        }
        painter.text(
            c + dir * (r_in + r_mid) / 2.0,
            Align2::CENTER_CENTER,
            minor_name(pc_major + 9),
            FontId::proportional((size * 0.042).clamp(8.5, 12.0)),
            text_col(chord_i == Some((i, false))),
        );
    }
    painter.text(
        c,
        Align2::CENTER_CENTER,
        "5ths →",
        FontId::proportional((size * 0.045).clamp(9.0, 12.0)),
        Color32::from_gray(140),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(major_name(10), "B♭");
        assert_eq!(major_name(7), "G");
        assert_eq!(minor_name(9), "Am");
        assert_eq!(minor_name(10), "B♭m");
        assert_eq!(minor_name(6), "F♯m");
        assert_eq!(signature(1), 1);
        assert_eq!(signature(11), -1);
    }
}
