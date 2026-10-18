//! Mesh export: STL, 3MF and OBJ, all in millimetres.
//!
//! - STL: universal, but one anonymous triangle soup with no units.
//! - 3MF: what modern slicers prefer (Prusa, Bambu, Cura). Stores units and
//!   keeps each body as its own object. It's a zip of XML files.
//! - OBJ: plain text, opens in Blender and most 3D tools.
//!
//! - STEP: exact geometry (true planes, cylinders, arcs) for other CAD tools.
//!   Written by the kernel from the real B-rep, not from these meshes.

use crate::model::Mesh;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Step,
    Stl,
    ThreeMf,
    Obj,
}

impl Format {
    pub const ALL: [Format; 4] = [Format::Step, Format::ThreeMf, Format::Stl, Format::Obj];

    pub fn extension(self) -> &'static str {
        match self {
            Format::Step => "step",
            Format::Stl => "stl",
            Format::ThreeMf => "3mf",
            Format::Obj => "obj",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Format::Step => "STEP (exact, for CAD)",
            Format::Stl => "STL mesh",
            Format::ThreeMf => "3MF (3D printing)",
            Format::Obj => "OBJ mesh",
        }
    }
}

/// A named body to export.
pub struct Body<'a> {
    pub name: String,
    pub mesh: &'a Mesh,
}

/// Mesh formats only; STEP comes from the kernel (`kernel::write_step`).
pub fn bytes(format: Format, bodies: &[Body]) -> Vec<u8> {
    match format {
        Format::Step => Vec::new(),
        Format::Stl => stl(bodies),
        Format::ThreeMf => three_mf(bodies),
        Format::Obj => obj(bodies).into_bytes(),
    }
}

pub fn write(path: &Path, format: Format, bodies: &[Body]) -> Result<(), String> {
    if format == Format::Step {
        return Err("STEP is written from the solid model, not a mesh".into());
    }
    std::fs::write(path, bytes(format, bodies)).map_err(|e| format!("couldn't write {}: {e}", path.display()))
}

// ---------- STL ----------

fn stl(bodies: &[Body]) -> Vec<u8> {
    let count: usize = bodies.iter().map(|b| b.mesh.tris.len()).sum();
    let mut out = Vec::with_capacity(84 + 50 * count);
    let mut header = [0u8; 80];
    let tag = b"Oxide360 binary STL, units: mm";
    header[..tag.len()].copy_from_slice(tag);
    out.extend_from_slice(&header);
    out.extend_from_slice(&(count as u32).to_le_bytes());
    for b in bodies {
        let m = b.mesh;
        for t in 0..m.tris.len() {
            let n = m.normal(t);
            let vs = m.tris[t].map(|i| m.positions[i as usize]);
            for v in std::iter::once(n).chain(vs) {
                for c in [v.x, v.y, v.z] {
                    out.extend_from_slice(&c.to_le_bytes());
                }
            }
            out.extend_from_slice(&0u16.to_le_bytes()); // attribute byte count
        }
    }
    out
}

// ---------- OBJ ----------

fn obj(bodies: &[Body]) -> String {
    use std::fmt::Write;
    let mut s = String::from("# Oxide360 OBJ export, units: mm\n");
    let mut base = 1; // OBJ indices are 1-based and global across objects
    for b in bodies {
        let _ = writeln!(s, "o {}", b.name);
        for p in &b.mesh.positions {
            let _ = writeln!(s, "v {} {} {}", p.x, p.y, p.z);
        }
        for t in &b.mesh.tris {
            let _ = writeln!(s, "f {} {} {}", t[0] as usize + base, t[1] as usize + base, t[2] as usize + base);
        }
        base += b.mesh.positions.len();
    }
    s
}

// ---------- 3MF ----------

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn three_mf_model(bodies: &[Body]) -> String {
    use std::fmt::Write;
    let mut s = String::from(concat!(
        r#"<?xml version="1.0" encoding="UTF-8"?>"#,
        "\n",
        r#"<model unit="millimeter" xml:lang="en-US" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">"#,
        "\n <resources>\n"
    ));
    for (k, b) in bodies.iter().enumerate() {
        let _ = writeln!(s, r#"  <object id="{}" type="model" name="{}">"#, k + 1, xml_escape(&b.name));
        s.push_str("   <mesh>\n    <vertices>\n");
        for p in &b.mesh.positions {
            let _ = writeln!(s, r#"     <vertex x="{}" y="{}" z="{}"/>"#, p.x, p.y, p.z);
        }
        s.push_str("    </vertices>\n    <triangles>\n");
        for t in &b.mesh.tris {
            let _ = writeln!(s, r#"     <triangle v1="{}" v2="{}" v3="{}"/>"#, t[0], t[1], t[2]);
        }
        s.push_str("    </triangles>\n   </mesh>\n  </object>\n");
    }
    s.push_str(" </resources>\n <build>\n");
    for k in 0..bodies.len() {
        let _ = writeln!(s, r#"  <item objectid="{}"/>"#, k + 1);
    }
    s.push_str(" </build>\n</model>\n");
    s
}

const CONTENT_TYPES: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8"?>"#,
    "\n",
    r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#,
    r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#,
    r#"<Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>"#,
    "</Types>\n"
);

const RELS: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8"?>"#,
    "\n",
    r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    r#"<Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>"#,
    "</Relationships>\n"
);

fn three_mf(bodies: &[Body]) -> Vec<u8> {
    zip_stored(&[
        ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
        ("_rels/.rels", RELS.as_bytes()),
        ("3D/3dmodel.model", three_mf_model(bodies).as_bytes()),
    ])
}

// ---------- minimal zip writer (stored, no compression) ----------

fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *slot = c;
    }
    !data.iter().fold(!0u32, |c, &b| table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8))
}

fn zip_stored(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    let le16 = |v: &mut Vec<u8>, x: u16| v.extend_from_slice(&x.to_le_bytes());
    let le32 = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_le_bytes());
    for (name, data) in files {
        let (crc, size, offset) = (crc32(data), data.len() as u32, out.len() as u32);
        // Local file header
        le32(&mut out, 0x0403_4b50);
        le16(&mut out, 20); // version needed
        le16(&mut out, 0); // flags
        le16(&mut out, 0); // method: stored
        le16(&mut out, 0); // mod time
        le16(&mut out, 0x21); // mod date: 1980-01-01
        le32(&mut out, crc);
        le32(&mut out, size);
        le32(&mut out, size);
        le16(&mut out, name.len() as u16);
        le16(&mut out, 0); // extra length
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        // Central directory entry
        le32(&mut central, 0x0201_4b50);
        le16(&mut central, 20); // version made by
        le16(&mut central, 20); // version needed
        le16(&mut central, 0);
        le16(&mut central, 0);
        le16(&mut central, 0);
        le16(&mut central, 0x21);
        le32(&mut central, crc);
        le32(&mut central, size);
        le32(&mut central, size);
        le16(&mut central, name.len() as u16);
        le16(&mut central, 0); // extra
        le16(&mut central, 0); // comment
        le16(&mut central, 0); // disk
        le16(&mut central, 0); // internal attrs
        le32(&mut central, 0); // external attrs
        le32(&mut central, offset);
        central.extend_from_slice(name.as_bytes());
    }
    let (cd_offset, cd_size) = (out.len() as u32, central.len() as u32);
    out.extend_from_slice(&central);
    // End of central directory
    le32(&mut out, 0x0605_4b50);
    le16(&mut out, 0);
    le16(&mut out, 0);
    le16(&mut out, files.len() as u16);
    le16(&mut out, files.len() as u16);
    le32(&mut out, cd_size);
    le32(&mut out, cd_offset);
    le16(&mut out, 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Extrude, build_extrude};
    use crate::sketch::{Anchor, Editor, OriginPlane, Sketch, Tool};
    use glam::DVec2;

    fn cylinder() -> Mesh {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Circle);
        ed.click(&mut sk, Anchor::Point(0));
        ed.click(&mut sk, Anchor::Free(DVec2::new(20.0, 20.0)));
        let ex = Extrude { op: Default::default(), name: "E".into(), sketch: 0, profiles: vec![0], distance: 30.0, extent: Default::default(), distance2: 0.0, taper: 0.0, target: None };
        build_extrude(&[sk], &ex).unwrap()
    }

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn stl_has_correct_layout() {
        let m = cylinder();
        let b = bytes(Format::Stl, &[Body { name: "Body1".into(), mesh: &m }]);
        assert_eq!(b.len(), 84 + 50 * m.tris.len());
        assert_eq!(u32::from_le_bytes(b[80..84].try_into().unwrap()) as usize, m.tris.len());
        let f = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let v0 = m.positions[m.tris[0][0] as usize];
        assert_eq!((f(84 + 12), f(84 + 16), f(84 + 20)), (v0.x, v0.y, v0.z));
    }

    #[test]
    fn obj_uses_global_one_based_indices() {
        let m = cylinder();
        let text = obj(&[Body { name: "A".into(), mesh: &m }, Body { name: "B".into(), mesh: &m }]);
        let n = m.positions.len();
        assert_eq!(text.lines().filter(|l| l.starts_with("v ")).count(), 2 * n);
        assert_eq!(text.lines().filter(|l| l.starts_with("f ")).count(), 2 * m.tris.len());
        let max_index: usize = text
            .lines()
            .filter(|l| l.starts_with("f "))
            .flat_map(|l| l[2..].split(' ').map(|x| x.parse::<usize>().unwrap()).collect::<Vec<_>>())
            .max()
            .unwrap();
        assert_eq!(max_index, 2 * n, "second body's faces point at its own vertices");
    }

    #[test]
    fn three_mf_zip_is_well_formed() {
        let m = cylinder();
        let z = bytes(Format::ThreeMf, &[Body { name: "Body <1>".into(), mesh: &m }]);
        // End-of-central-directory record says 3 entries.
        let eocd = z.len() - 22;
        assert_eq!(u32::from_le_bytes(z[eocd..eocd + 4].try_into().unwrap()), 0x0605_4b50);
        assert_eq!(u16::from_le_bytes(z[eocd + 10..eocd + 12].try_into().unwrap()), 3);
        let model = three_mf_model(&[Body { name: "Body <1>".into(), mesh: &m }]);
        assert!(model.contains(r#"unit="millimeter""#));
        assert!(model.contains("Body &lt;1&gt;"), "names are XML-escaped");
        assert_eq!(model.matches("<vertex ").count(), m.positions.len());
        assert_eq!(model.matches("<triangle ").count(), m.tris.len());
    }
}
