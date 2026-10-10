//! Mesh exports for printing and other tools: 3MF (what slicers prefer:
//! units, one object per body) and Wavefront OBJ.

use std::io::Write;
use std::path::Path;

use fission_kernel::Mesh;

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The 3MF model document: one mesh object per body, all placed in the build.
fn model_xml(bodies: &[(String, Mesh)]) -> String {
    let mut x = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <model unit=\"millimeter\" xml:lang=\"en-US\" xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\">\n\
         <metadata name=\"Application\">Fission</metadata>\n<resources>\n",
    );
    for (i, (name, mesh)) in bodies.iter().enumerate() {
        x += &format!("<object id=\"{}\" type=\"model\" name=\"{}\"><mesh><vertices>\n", i + 1, xml_escape(name));
        for p in &mesh.positions {
            x += &format!("<vertex x=\"{}\" y=\"{}\" z=\"{}\"/>\n", p[0], p[1], p[2]);
        }
        x += "</vertices><triangles>\n";
        for t in mesh.indices.chunks_exact(3) {
            x += &format!("<triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"/>\n", t[0], t[1], t[2]);
        }
        x += "</triangles></mesh></object>\n";
    }
    x += "</resources>\n<build>\n";
    for i in 0..bodies.len() {
        x += &format!("<item objectid=\"{}\"/>\n", i + 1);
    }
    x += "</build>\n</model>\n";
    x
}

pub fn write_3mf(path: &Path, bodies: &[(String, Mesh)]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut put = |name: &str, body: &str| -> Result<(), String> {
        zip.start_file(name, options).map_err(|e| e.to_string())?;
        zip.write_all(body.as_bytes()).map_err(|e| e.to_string())
    };
    put(
        "[Content_Types].xml",
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"model\" ContentType=\"application/vnd.ms-package.3dmanufacturing-3dmodel+xml\"/>\
         </Types>\n",
    )?;
    put(
        "_rels/.rels",
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Target=\"/3D/3dmodel.model\" Id=\"rel0\" Type=\"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel\"/>\
         </Relationships>\n",
    )?;
    put("3D/3dmodel.model", &model_xml(bodies))?;
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn write_obj(path: &Path, bodies: &[(String, Mesh)]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut w = std::io::BufWriter::new(file);
    let mut out = || -> std::io::Result<()> {
        writeln!(w, "# Fission export, millimetres")?;
        let mut base = 1usize;
        for (name, mesh) in bodies {
            writeln!(w, "o {}", name.replace(char::is_whitespace, "_"))?;
            for p in &mesh.positions {
                writeln!(w, "v {} {} {}", p[0], p[1], p[2])?;
            }
            for n in &mesh.normals {
                writeln!(w, "vn {} {} {}", n[0], n[1], n[2])?;
            }
            let normals = mesh.normals.len() == mesh.positions.len();
            for t in mesh.indices.chunks_exact(3) {
                let [a, b, c] = [t[0] as usize + base, t[1] as usize + base, t[2] as usize + base];
                if normals {
                    writeln!(w, "f {a}//{a} {b}//{b} {c}//{c}")?;
                } else {
                    writeln!(w, "f {a} {b} {c}")?;
                }
            }
            base += mesh.positions.len();
        }
        w.flush()
    };
    out().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangle() -> Mesh {
        Mesh {
            positions: vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            indices: vec![0, 1, 2],
            ..Default::default()
        }
    }

    #[test]
    fn three_mf_has_the_parts_slicers_read() {
        let dir = std::env::temp_dir().join("fission-export-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.3mf");
        write_3mf(&path, &[("Body1".into(), triangle()), ("A & B".into(), triangle())]).unwrap();
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
        let mut model = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("3D/3dmodel.model").unwrap(), &mut model).unwrap();
        assert!(model.contains("unit=\"millimeter\""));
        assert_eq!(model.matches("<object ").count(), 2);
        assert!(model.contains("name=\"A &amp; B\""));
        assert!(model.contains("<triangle v1=\"0\" v2=\"1\" v3=\"2\"/>"));
        assert!(zip.by_name("[Content_Types].xml").is_ok() && zip.by_name("_rels/.rels").is_ok());
    }

    #[test]
    fn obj_indices_continue_across_bodies() {
        let dir = std::env::temp_dir().join("fission-export-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.obj");
        write_obj(&path, &[("One".into(), triangle()), ("Two".into(), triangle())]).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("o One") && text.contains("o Two"));
        assert!(text.contains("f 4//4 5//5 6//6"), "second body's faces start after the first's vertices");
    }
}
