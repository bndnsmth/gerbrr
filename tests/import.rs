use gerbrr::document;
use std::io::Write;
use std::path::{Path, PathBuf};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/test-imports")
            .join(format!("{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn folder_and_zip_load_identical_geometry_without_extraction() {
    let fixture = Fixture::new();
    let folder = fixture.0.join("board");
    std::fs::create_dir(&folder).unwrap();
    let archive = fixture.0.join("board.zip");
    let file = std::fs::File::create(&archive).unwrap();
    let mut writer = zip::ZipWriter::new(file);
    for (name, source) in document::demo_sources() {
        std::fs::write(folder.join(&name), &source).unwrap();
        writer
            .start_file(
                format!("../never-extract/{name}"),
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
        writer.write_all(source.as_bytes()).unwrap();
    }
    writer.finish().unwrap();
    let a = document::load(vec![folder]).unwrap();
    let b = document::load(vec![archive]).unwrap();
    assert_eq!(a.layers.len(), 6);
    assert_eq!(a.primitives(), b.primitives());
    assert_eq!(a.triangles(), b.triangles());
    assert_eq!(a.bounds, b.bounds);
    assert!(!fixture.0.join("never-extract").exists());
    assert_eq!(b.warning_count(), 0);
}

#[test]
fn uppercase_drill_extension_without_header_is_recognized() {
    let fixture = Fixture::new();
    let file = fixture.0.join("holes.DRD");
    std::fs::write(&file, "METRIC,TZ\nT1C1.0\nT1\nX1.0Y2.0\nM30\n").unwrap();
    let doc = document::load(vec![file]).unwrap();
    assert_eq!(doc.layers[0].kind, document::LayerKind::Drill);
    assert_eq!(doc.primitives(), 1);
}

#[test]
fn explicitly_selected_unusual_extension_can_be_opened() {
    let fixture = Fixture::new();
    let file = fixture.0.join("artwork.unusual");
    std::fs::write(&file, "%FSLAX36Y36*%%MOMM*%%ADD10C,1*%D10*X0Y0D03*M02*").unwrap();
    assert_eq!(document::load(vec![file]).unwrap().primitives(), 1);
}

#[test]
fn empty_and_corrupt_inputs_are_actionable_errors() {
    let fixture = Fixture::new();
    assert!(
        document::load(vec![fixture.0.clone()])
            .unwrap_err_string()
            .contains("No Gerber")
    );
    let archive = fixture.0.join("broken.zip");
    std::fs::write(&archive, "not a zip").unwrap();
    assert!(
        document::load(vec![archive])
            .unwrap_err_string()
            .contains("Invalid ZIP")
    );
}

#[test]
fn native_recursive_import_keeps_nc_settings_scoped_to_their_directory() {
    let fixture = Fixture::new();
    let settings = "INTEGER-PLACES 2\nDECIMAL-PLACES 5\nOUTPUT-UNITS ENGLISH\nSUPPRESS-LEAD-ZEROES NO\nSUPPRESS-TRAIL-ZEROES NO\n";
    let drill = "M48\nINCH\nT1C.01\n%\nT1\nX100000Y200000\nM30";
    for folder in ["a", "b"] {
        std::fs::create_dir(fixture.0.join(folder)).unwrap();
        std::fs::write(fixture.0.join(folder).join("holes.drl"), drill).unwrap();
    }
    std::fs::write(fixture.0.join("a/nc_param.txt"), settings).unwrap();
    let doc = document::load(vec![fixture.0.clone()]).unwrap();
    assert_eq!(doc.layers.len(), 2);
    assert_eq!(doc.warning_count(), 1);
    for layer in &doc.layers {
        let expected = match Path::new(&layer.name).parent().and_then(Path::to_str) {
            Some("a") => 25.4,
            Some("b") => 254.0,
            _ => panic!("Unexpected imported path: {}", layer.name),
        };
        let actual = layer.geometry.bounds.center()[0];
        assert!(
            (actual - expected).abs() < 0.002,
            "{}: expected X {expected}, got {actual}",
            layer.name
        );
    }
}

trait ErrorString {
    fn unwrap_err_string(self) -> String;
}
impl ErrorString for Result<document::Document, String> {
    fn unwrap_err_string(self) -> String {
        match self {
            Err(e) => e,
            Ok(_) => panic!("Expected import to fail"),
        }
    }
}
