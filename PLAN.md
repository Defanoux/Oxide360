# Render mode with appearances

- [ ] 1. NEW src/appearance.rs + one `mod appearance;` line in src/main.rs.
      `pub enum Appearance { Default, Metal, Plastic, Rubber, Paint }` with
      `pub fn params(self) -> (f32, f32)` returning (metalness, gloss), 0.0–1.0.
      Add a test that every variant's params are within 0.0–1.0.
      Verify: cargo test appearance
- [ ] 2. src/gpu.rs: add `Style::Rendered`, label "Rendered", add it to `Style::ALL` (size 5).
      In every `match style`, treat Rendered exactly like Shaded for now.
      Verify: cargo test visual_styles
- [ ] 3. src/gpu.rs: read lines 20–60 (the WGSL). Add a per-vertex `shine: vec2<f32>`
      (metalness, gloss), filled with (0, 0) everywhere. No visual change yet.
      Verify: cargo test --bin oxide360 gpu
