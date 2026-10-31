//! Obrazy do panelu podglądu dla plików, których Windows nie pokazuje: SVG (resvg), próbka
//! czcionki (ab_glyph) i model 3D STL/OBJ (prosty renderer z cieniowaniem, bez GPU).
//! Wynik: (szerokość, wysokość, piksele RGBA).

use std::path::Path;

pub type Rgba = (u32, u32, Vec<u8>);

/// SVG dopasowany do szerokości `w` (wysokość z proporcji, do 2 × szerokości).
pub fn svg(path: &str, w: u32) -> Option<Rgba> {
    let data = std::fs::read(path).ok()?;
    let mut opt = resvg::usvg::Options::default();
    opt.fontdb_mut().load_system_fonts();
    let tree = resvg::usvg::Tree::from_data(&data, &opt).ok()?;
    let size = tree.size();
    let scale = w as f32 / size.width().max(1.0);
    let h = ((size.height() * scale) as u32).clamp(1, w * 2);
    let mut pix = resvg::tiny_skia::Pixmap::new(w.max(1), h)?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pix.as_mut());
    // tiny-skia trzyma kolory przemnożone przez alfę – Slint chce zwykłe.
    let mut px = pix.take();
    for p in px.chunks_mut(4) {
        let a = p[3] as u32;
        if a > 0 && a < 255 {
            for c in &mut p[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    Some((w, h, px))
}

/// Próbka czcionki: nazwa pliku, alfabet z polskimi znakami i kilka rozmiarów, na przezroczystym tle.
pub fn font(path: &str, w: u32, color: [u8; 3]) -> Option<Rgba> {
    use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
    let data = std::fs::read(path).ok()?;
    let font = FontRef::try_from_slice(&data).ok()?;
    let lines: [(&str, f32); 6] = [
        (crate::lang::tr("Aa Bb Cc"), 56.0),
        (crate::lang::tr("Zażółć gęślą jaźń"), 30.0),
        ("ABCDEFGHIJKLMNOPRSTUWYZ", 20.0),
        ("abcdefghijklmnoprstuwyz", 20.0),
        ("0123456789 !?&@%(),.", 20.0),
        (crate::lang::tr("Pchnąć w tę łódź jeża lub ośm skrzyń fig."), 14.0),
    ];
    let pad = 8.0;
    let h: f32 = lines.iter().map(|(_, s)| s * 1.35).sum::<f32>() + pad * 2.0;
    let (wd, ht) = (w.max(1), h as u32);
    let mut px = vec![0u8; (wd * ht * 4) as usize];
    let mut y = pad;
    for (text, size) in lines {
        let sf = font.as_scaled(PxScale::from(size));
        let mut x = pad;
        let base = y + sf.ascent();
        let mut prev = None;
        for ch in text.chars() {
            let id = sf.glyph_id(ch);
            if let Some(p) = prev {
                x += sf.kern(p, id);
            }
            let g = id.with_scale_and_position(PxScale::from(size), ab_glyph::point(x, base));
            x += sf.h_advance(id);
            prev = Some(id);
            if let Some(o) = font.outline_glyph(g) {
                let b = o.px_bounds();
                o.draw(|gx, gy, cov| {
                    let (px_x, px_y) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                    if px_x < 0 || px_y < 0 || px_x >= wd as i32 || px_y >= ht as i32 {
                        return;
                    }
                    let i = ((px_y as u32 * wd + px_x as u32) * 4) as usize;
                    let a = (cov.clamp(0.0, 1.0) * 255.0) as u8;
                    if a > px[i + 3] {
                        px[i..i + 3].copy_from_slice(&color);
                        px[i + 3] = a;
                    }
                });
            }
        }
        y += size * 1.35;
    }
    Some((wd, ht, px))
}

type V3 = [f32; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn norm(a: V3) -> V3 {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt().max(1e-9);
    [a[0] / l, a[1] / l, a[2] / l]
}

/// Trójkąty z pliku STL (binarny albo tekstowy).
fn stl(data: &[u8]) -> Vec<[V3; 3]> {
    let f = |b: &[u8], o: usize| f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    if data.len() >= 84 {
        let n = u32::from_le_bytes([data[80], data[81], data[82], data[83]]) as usize;
        if 84 + n * 50 == data.len() {
            return (0..n)
                .map(|i| {
                    let o = 84 + i * 50 + 12;
                    [[f(data, o), f(data, o + 4), f(data, o + 8)], [f(data, o + 12), f(data, o + 16), f(data, o + 20)], [f(data, o + 24), f(data, o + 28), f(data, o + 32)]]
                })
                .collect();
        }
    }
    let text = String::from_utf8_lossy(data);
    let verts: Vec<V3> = text
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            if it.next()? != "vertex" {
                return None;
            }
            Some([it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?])
        })
        .collect();
    verts.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect()
}

/// Trójkąty z pliku OBJ (ściany wielokątne rozbite na wachlarz trójkątów).
fn obj(data: &[u8]) -> Vec<[V3; 3]> {
    let text = String::from_utf8_lossy(data);
    let mut v: Vec<V3> = Vec::new();
    let mut tris = Vec::new();
    for l in text.lines() {
        let mut it = l.split_whitespace();
        match it.next() {
            Some("v") => {
                let p: Vec<f32> = it.take(3).filter_map(|x| x.parse().ok()).collect();
                if p.len() == 3 {
                    v.push([p[0], p[1], p[2]]);
                }
            }
            Some("f") => {
                let idx: Vec<usize> = it
                    .filter_map(|x| {
                        let i: i64 = x.split('/').next()?.parse().ok()?;
                        Some(if i < 0 { (v.len() as i64 + i) as usize } else { (i - 1) as usize })
                    })
                    .collect();
                for k in 1..idx.len().saturating_sub(1) {
                    if let (Some(&a), Some(&b), Some(&c)) = (v.get(idx[0]), v.get(idx[k]), v.get(idx[k + 1])) {
                        tris.push([a, b, c]);
                    }
                }
            }
            _ => {}
        }
    }
    tris
}

/// Model 3D (STL, OBJ) w widoku izometrycznym, cieniowany światłem z góry z lewej, kwadrat `size`.
pub fn model(path: &str, size: u32, color: [u8; 3]) -> Option<Rgba> {
    let data = std::fs::read(path).ok()?;
    let ext = Path::new(path).extension()?.to_string_lossy().to_lowercase();
    let tris = match ext.as_str() {
        "stl" => stl(&data),
        "obj" => obj(&data),
        _ => return None,
    };
    if tris.is_empty() {
        return None;
    }
    // Obrót: 35° wokół osi pionowej, 25° w dół; oś Z modeli do druku to „góra”.
    let (ya, pa) = (35f32.to_radians(), 25f32.to_radians());
    let up_z = ext == "stl";
    let rot = |p: V3| -> V3 {
        let p = if up_z { [p[0], p[2], -p[1]] } else { p };
        let x = p[0] * ya.cos() + p[2] * ya.sin();
        let z = -p[0] * ya.sin() + p[2] * ya.cos();
        let y = p[1] * pa.cos() - z * pa.sin();
        let z = p[1] * pa.sin() + z * pa.cos();
        [x, y, z]
    };
    let tris: Vec<[V3; 3]> = tris.iter().map(|t| [rot(t[0]), rot(t[1]), rot(t[2])]).collect();
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for t in &tris {
        for p in t {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
    }
    let span = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1e-6);
    let s = size as f32 * 0.86 / span;
    let (cx, cy) = ((lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0);
    let n = size as usize;
    let mut zbuf = vec![f32::MIN; n * n];
    let mut px = vec![0u8; n * n * 4];
    let light = norm([-0.4, 0.8, 0.6]);
    for t in &tris {
        let nrm = norm(cross(sub(t[1], t[0]), sub(t[2], t[0])));
        // Obie strony ściany – modele bywają niedomknięte albo z odwróconymi normalnymi.
        let lambert = (nrm[0] * light[0] + nrm[1] * light[1] + nrm[2] * light[2]).abs();
        let shade = 0.28 + 0.72 * lambert;
        let sc = |p: V3| ((p[0] - cx) * s + size as f32 / 2.0, size as f32 / 2.0 - (p[1] - cy) * s, p[2]);
        let (a, b, c) = (sc(t[0]), sc(t[1]), sc(t[2]));
        let (minx, maxx) = (a.0.min(b.0).min(c.0).floor().max(0.0) as usize, a.0.max(b.0).max(c.0).ceil().min(size as f32 - 1.0) as usize);
        let (miny, maxy) = (a.1.min(b.1).min(c.1).floor().max(0.0) as usize, a.1.max(b.1).max(c.1).ceil().min(size as f32 - 1.0) as usize);
        let area = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
        if area.abs() < 1e-9 || minx > maxx || miny > maxy {
            continue;
        }
        for y in miny..=maxy {
            for x in minx..=maxx {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 = ((b.0 - fx) * (c.1 - fy) - (b.1 - fy) * (c.0 - fx)) / area;
                let w1 = ((c.0 - fx) * (a.1 - fy) - (c.1 - fy) * (a.0 - fx)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let z = w0 * a.2 + w1 * b.2 + w2 * c.2;
                let i = y * n + x;
                if z > zbuf[i] {
                    zbuf[i] = z;
                    let o = i * 4;
                    for k in 0..3 {
                        px[o + k] = (color[k] as f32 * shade).min(255.0) as u8;
                    }
                    px[o + 3] = 255;
                }
            }
        }
    }
    Some((size, size, px))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_obj_renders() {
        let p = std::env::temp_dir().join(format!("pd-cube-{}.obj", std::process::id()));
        std::fs::write(&p, "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv 0 0 1\nv 1 0 1\nv 1 1 1\nv 0 1 1\nf 1 2 3 4\nf 5 6 7 8\nf 1 2 6 5\nf 2 3 7 6\nf 3 4 8 7\nf 4 1 5 8\n").unwrap();
        let (w, h, px) = model(&p.to_string_lossy(), 64, [200, 200, 200]).unwrap();
        assert_eq!((w, h), (64, 64));
        assert!(px.chunks(4).filter(|c| c[3] == 255).count() > 500);
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn svg_renders() {
        let p = std::env::temp_dir().join(format!("pd-{}.svg", std::process::id()));
        std::fs::write(&p, r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="5"><rect width="10" height="5" fill="red"/></svg>"#).unwrap();
        let (w, h, px) = svg(&p.to_string_lossy(), 100).unwrap();
        assert_eq!((w, h), (100, 50));
        assert_eq!(&px[..4], &[255, 0, 0, 255]);
        let _ = std::fs::remove_file(p);
    }
}
