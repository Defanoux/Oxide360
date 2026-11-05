//! A design's preview: a small square picture of its bodies in their colours,
//! seen from the home angle (front-right-top) and fitted whole. Drawn on the
//! CPU (plain shading, no materials — quick at save time, no GPU needed) and
//! kept in the design's file beside the document, so the file viewer can
//! show it on any computer without building the model.

use crate::model::Mesh;
use glam::Vec3;

/// The preview's side in pixels.
pub const SIZE: usize = 256;

/// Drawn this many times bigger, then shrunk: smooth edges.
const SUPER: usize = 2;

/// RGBA pixels (`SIZE` × `SIZE`, clear where there's nothing) of `bodies`
/// (each a mesh and its colour).
pub fn render(bodies: &[(&Mesh, [u8; 3])]) -> Vec<u8> {
    let n = SIZE * SUPER;
    // Looking from front-right-top (front faces −Y), Z up.
    let forward = Vec3::new(-1.0, 1.0, -0.75).normalize();
    let right = forward.cross(Vec3::Z).normalize();
    let up = right.cross(forward);
    let light = (-forward * 0.6 + up * 0.5 - right * 0.3).normalize();
    // Fit: the projected bounds, with a margin, centred.
    let (mut lo, mut hi) = (glam::Vec2::splat(f32::MAX), glam::Vec2::splat(f32::MIN));
    for (m, _) in bodies {
        for &p in &m.positions {
            let q = glam::Vec2::new(p.dot(right), p.dot(up));
            lo = lo.min(q);
            hi = hi.max(q);
        }
    }
    let mut rgba = vec![0u8; SIZE * SIZE * 4];
    if lo.x > hi.x {
        return rgba; // nothing to draw
    }
    let span = (hi - lo).max_element().max(1e-6);
    let scale = n as f32 * 0.88 / span;
    let mid = (lo + hi) * 0.5;
    let to_px = |p: Vec3| {
        let q = (glam::Vec2::new(p.dot(right), p.dot(up)) - mid) * scale;
        (n as f32 / 2.0 + q.x, n as f32 / 2.0 - q.y, p.dot(forward))
    };
    let mut depth = vec![f32::MAX; n * n];
    let mut color = vec![[0f32; 4]; n * n];
    for (m, c) in bodies {
        let base = Vec3::new(c[0] as f32, c[1] as f32, c[2] as f32) / 255.0;
        for t in &m.tris {
            let [a, b, cc] = [m.positions[t[0] as usize], m.positions[t[1] as usize], m.positions[t[2] as usize]];
            let normal = (b - a).cross(cc - a);
            if normal.length_squared() < 1e-20 {
                continue;
            }
            let mut normal = normal.normalize();
            if normal.dot(forward) > 0.0 {
                normal = -normal; // (lit from the side we see)
            }
            let shade = 0.32 + 0.55 * normal.dot(-forward).max(0.0) + 0.3 * normal.dot(light).max(0.0);
            let col = (base * shade).min(Vec3::ONE);
            fill(&mut depth, &mut color, n, to_px(a), to_px(b), to_px(cc), [col.x, col.y, col.z, 1.0]);
        }
    }
    // Shrink: each pixel the average of its SUPER × SUPER block.
    for y in 0..SIZE {
        for x in 0..SIZE {
            let mut sum = [0f32; 4];
            for dy in 0..SUPER {
                for dx in 0..SUPER {
                    let c = color[(y * SUPER + dy) * n + x * SUPER + dx];
                    for k in 0..3 {
                        sum[k] += c[k] * c[3];
                    }
                    sum[3] += c[3];
                }
            }
            let cover = sum[3] / (SUPER * SUPER) as f32;
            let o = (y * SIZE + x) * 4;
            if sum[3] > 0.0 {
                for k in 0..3 {
                    rgba[o + k] = (sum[k] / sum[3] * 255.0).round() as u8;
                }
            }
            rgba[o + 3] = (cover * 255.0).round() as u8;
        }
    }
    rgba
}

/// Fill a triangle (pixel x, y and depth at each corner), nearest wins.
fn fill(depth: &mut [f32], color: &mut [[f32; 4]], n: usize, a: (f32, f32, f32), b: (f32, f32, f32), c: (f32, f32, f32), col: [f32; 4]) {
    let area = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
    if area.abs() < 1e-9 {
        return;
    }
    let x0 = a.0.min(b.0).min(c.0).floor().max(0.0) as usize;
    let x1 = (a.0.max(b.0).max(c.0).ceil() as usize).min(n - 1);
    let y0 = a.1.min(b.1).min(c.1).floor().max(0.0) as usize;
    let y1 = (a.1.max(b.1).max(c.1).ceil() as usize).min(n - 1);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let w0 = ((b.0 - px) * (c.1 - py) - (b.1 - py) * (c.0 - px)) / area;
            let w1 = ((c.0 - px) * (a.1 - py) - (c.1 - py) * (a.0 - px)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let z = w0 * a.2 + w1 * b.2 + w2 * c.2;
            let i = y * n + x;
            if z < depth[i] {
                depth[i] = z;
                color[i] = col;
            }
        }
    }
}

/// The preview as PNG bytes.
pub fn png(rgba: &[u8]) -> Vec<u8> {
    crate::control::png(SIZE, SIZE, rgba)
}

/// PNG bytes back to (width, height, RGBA).
pub fn decode_png(bytes: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    buf.truncate(info.buffer_size());
    // (Grey or RGB without alpha come back as such: widen to RGBA.)
    let px = buf.len() / (w * h).max(1);
    let rgba = match px {
        4 => buf,
        3 => buf.chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        2 => buf.chunks(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        _ => return None,
    };
    Some((w, h, rgba))
}

/// Base64 (as `control::base64` writes it) back to bytes.
pub fn unbase64(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let bytes: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let pad = chunk.iter().filter(|&&c| c == b'=').count();
        let mut v = 0u32;
        for (k, &c) in chunk.iter().enumerate() {
            v |= if c == b'=' { 0 } else { val(c)? } << (18 - 6 * k);
        }
        let got = [(v >> 16) as u8, (v >> 8) as u8, v as u8];
        out.extend_from_slice(&got[..3 - pad.min(2)]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(at: Vec3, s: f32) -> Mesh {
        let p: Vec<Vec3> = (0..8).map(|i| at + Vec3::new((i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32) * s).collect();
        let quads = [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]];
        let tris = quads.iter().flat_map(|q| [[q[0], q[1], q[2]], [q[0], q[2], q[3]]]).collect();
        Mesh { positions: p, tris }
    }

    #[test]
    fn a_body_fills_the_middle_in_its_colour_and_fits_whatever_its_size() {
        for s in [0.5f32, 30.0, 4000.0] {
            let m = cube(Vec3::splat(100.0), s);
            let img = render(&[(&m, [200, 60, 40])]);
            assert_eq!(img.len(), SIZE * SIZE * 4);
            let at = |x: usize, y: usize| &img[(y * SIZE + x) * 4..(y * SIZE + x) * 4 + 4];
            let mid = at(SIZE / 2, SIZE / 2);
            assert_eq!(mid[3], 255, "solid in the middle ({s})");
            assert!(mid[0] > mid[1] && mid[0] > mid[2], "red stays red: {mid:?}");
            assert_eq!(at(2, 2)[3], 0, "clear in the corner");
            // Fitted: it reaches near the edge on its widest side, not past it.
            let rows_with: Vec<usize> = (0..SIZE).filter(|&y| (0..SIZE).any(|x| at(x, y)[3] > 0)).collect();
            let cols_with: Vec<usize> = (0..SIZE).filter(|&x| (0..SIZE).any(|y| at(x, y)[3] > 0)).collect();
            let wide = (cols_with.len().max(rows_with.len())) as f32 / SIZE as f32;
            assert!((0.8..0.95).contains(&wide), "{wide} of the square ({s})");
        }
        assert!(render(&[]).iter().all(|&b| b == 0), "nothing: clear");
    }

    #[test]
    fn the_nearer_body_hides_the_one_behind() {
        // A green cube right in front (−Y, toward the viewer) of a blue one.
        let back = cube(Vec3::new(0.0, 0.0, 0.0), 10.0);
        let front = cube(Vec3::new(3.0, -12.0, 3.0), 4.0);
        let img = render(&[(&back, [40, 60, 220]), (&front, [40, 220, 60])]);
        let greens = img.chunks(4).filter(|p| p[3] == 255 && p[1] > p[2]).count();
        let blues = img.chunks(4).filter(|p| p[3] == 255 && p[2] > p[1]).count();
        assert!(greens > 100 && blues > greens, "{greens} green, {blues} blue");
    }

    #[test]
    fn png_and_base64_go_there_and_back() {
        let img = render(&[(&cube(Vec3::ZERO, 1.0), [10, 20, 30])]);
        let bytes = png(&img);
        assert_eq!(decode_png(&bytes), Some((SIZE, SIZE, img)));
        for data in [&b""[..], b"a", b"ab", b"abc", b"abcd", &bytes[..100]] {
            assert_eq!(unbase64(&crate::control::base64(data)).as_deref(), Some(data));
        }
        assert_eq!(unbase64("a?bc"), None);
    }
}
