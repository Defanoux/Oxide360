//! Test-only: draw a piece of UI headlessly to a PNG, so a dialog's layout can
//! be looked at without the app (egui's triangles rasterised on the CPU, the
//! PNG written uncompressed — no extra crates).

use eframe::egui;

/// Run `ui` for a few frames in a `size` window and save what it draws.
pub fn shot(path: &str, size: [u32; 2], mut ui_fn: impl FnMut(&mut egui::Ui)) {
    let ctx = egui::Context::default();
    let mut fonts: std::collections::HashMap<egui::TextureId, egui::ColorImage> = Default::default();
    let mut last = None;
    for _ in 0..3 {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size[0] as f32, size[1] as f32))),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            egui::Frame::new().fill(egui::Color32::from_gray(27)).inner_margin(10.0).show(ui, |ui| ui_fn(ui));
        });
        for (id, deltas) in std::mem::take(&mut out.textures_delta.set) {
            for delta in deltas {
                let img = match delta.image {
                    egui::ImageData::Color(c) => (*c).clone(),
                };
                match delta.pos {
                    None => {
                        fonts.insert(id, img);
                    }
                    Some([x, y]) => {
                        let t = fonts.get_mut(&id).expect("texture made before it's patched");
                        for row in 0..img.size[1] {
                            for col in 0..img.size[0] {
                                t.pixels[(y + row) * t.size[0] + x + col] = img.pixels[row * img.size[0] + col];
                            }
                        }
                    }
                }
            }
        }
        out.textures_delta.clear();
        last = Some(ctx.tessellate(out.shapes, 1.0));
    }
    let (w, h) = (size[0] as usize, size[1] as usize);
    let mut px = vec![[27.0f32, 27.0, 27.0]; w * h];
    for prim in last.unwrap_or_default() {
        let egui::epaint::Primitive::Mesh(mesh) = prim.primitive else { continue };
        let clip = prim.clip_rect;
        let tex = fonts.get(&mesh.texture_id);
        for tri in mesh.indices.chunks(3) {
            let v = [&mesh.vertices[tri[0] as usize], &mesh.vertices[tri[1] as usize], &mesh.vertices[tri[2] as usize]];
            let (a, b, c) = (v[0].pos, v[1].pos, v[2].pos);
            let area = (b - a).x * (c - a).y - (b - a).y * (c - a).x;
            if area.abs() < 1e-6 {
                continue;
            }
            let lo = a.min(b).min(c).max(clip.min).max(egui::Pos2::ZERO);
            let hi = a.max(b).max(c).min(clip.max).min(egui::pos2(w as f32 - 1.0, h as f32 - 1.0));
            for y in lo.y.floor() as usize..=hi.y.ceil().max(0.0) as usize {
                for x in lo.x.floor() as usize..=hi.x.ceil().max(0.0) as usize {
                    if x >= w || y >= h {
                        continue;
                    }
                    let p = egui::pos2(x as f32 + 0.5, y as f32 + 0.5);
                    let w0 = ((b - p).x * (c - p).y - (b - p).y * (c - p).x) / area;
                    let w1 = ((c - p).x * (a - p).y - (c - p).y * (a - p).x) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                        continue;
                    }
                    let mix = |f: &dyn Fn(&egui::epaint::Vertex) -> f32| w0 * f(v[0]) + w1 * f(v[1]) + w2 * f(v[2]);
                    let col = [mix(&|q| q.color.r() as f32), mix(&|q| q.color.g() as f32), mix(&|q| q.color.b() as f32), mix(&|q| q.color.a() as f32)];
                    let texel = tex.map_or([255.0; 4], |t| {
                        let (u, vv) = (mix(&|q| q.uv.x), mix(&|q| q.uv.y));
                        let tx = ((u * t.size[0] as f32) as usize).min(t.size[0] - 1);
                        let ty = ((vv * t.size[1] as f32) as usize).min(t.size[1] - 1);
                        let c = t.pixels[ty * t.size[0] + tx];
                        [c.r() as f32, c.g() as f32, c.b() as f32, c.a() as f32]
                    });
                    // Premultiplied alpha.
                    let src = [col[0] * texel[0] / 255.0, col[1] * texel[1] / 255.0, col[2] * texel[2] / 255.0];
                    let alpha = col[3] * texel[3] / 255.0 / 255.0;
                    let d = &mut px[y * w + x];
                    for k in 0..3 {
                        d[k] = src[k] + d[k] * (1.0 - alpha);
                    }
                }
            }
        }
    }
    let rgb: Vec<u8> = px.iter().flat_map(|p| p.map(|c| c.clamp(0.0, 255.0) as u8)).collect();
    std::fs::write(path, png(size[0], size[1], &rgb)).expect("write the shot");
}

/// An RGB PNG, stored (uncompressed) deflate blocks.
fn png(w: u32, h: u32, rgb: &[u8]) -> Vec<u8> {
    fn crc(data: &[u8]) -> u32 {
        let mut c = 0xffff_ffffu32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    let chunk = |out: &mut Vec<u8>, kind: &[u8], data: &[u8]| {
        out.extend((data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend(crc(&body).to_be_bytes());
    };
    let mut raw = Vec::new();
    for row in rgb.chunks(w as usize * 3) {
        raw.push(0); // no filter
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, b) in blocks.iter().enumerate() {
        z.push(if i + 1 == blocks.len() { 1 } else { 0 });
        z.extend((b.len() as u16).to_le_bytes());
        z.extend((!(b.len() as u16)).to_le_bytes());
        z.extend_from_slice(b);
    }
    let (mut s1, mut s2) = (1u32, 0u32);
    for &b in &raw {
        s1 = (s1 + b as u32) % 65521;
        s2 = (s2 + s1) % 65521;
    }
    z.extend(((s2 << 16) | s1).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = Vec::new();
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}
