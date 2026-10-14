use gerbrr::{document, transfer};
use std::io::{Cursor, Write};

#[test]
fn byte_upload_matches_native_demo_and_survives_worker_transfer() {
    let files = document::demo_sources()
        .into_iter()
        .map(|(name, source)| (name, source.into_bytes()))
        .collect();
    let uploaded = document::load_bytes(files).unwrap();
    let decoded = transfer::decode(&transfer::encode(&uploaded).unwrap()).unwrap();
    let demo = document::demo();
    assert_eq!(decoded.layers.len(), demo.layers.len());
    assert_eq!(decoded.bounds, demo.bounds);
    assert_eq!(decoded.triangles(), demo.triangles());
    assert_eq!(decoded.warning_count(), 0);
}

#[test]
fn zip_upload_needs_no_filesystem_and_ignores_mac_resource_forks() {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, source) in document::demo_sources() {
        writer
            .start_file(
                format!("Gerbers/{name}"),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.write_all(source.as_bytes()).unwrap();
    }
    writer
        .start_file(
            "__MACOSX/._top.gtl",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    writer.write_all(b"not a Gerber").unwrap();
    let bytes = writer.finish().unwrap().into_inner();
    let doc = document::load_bytes(vec![("board.zip".into(), bytes)]).unwrap();
    assert_eq!(doc.layers.len(), 6);
    assert_eq!(doc.warning_count(), 0);
}

#[test]
fn folder_upload_ignores_non_fabrication_files() {
    let mut files: Vec<_> = document::demo_sources()
        .into_iter()
        .map(|(name, source)| (format!("board/{name}"), source.into_bytes()))
        .collect();
    files.push(("board/odbjob.tgz".into(), vec![0, 0xff, 0xaa]));
    assert_eq!(document::load_bytes(files).unwrap().layers.len(), 6);
}

#[test]
fn invalid_uploads_return_actionable_errors() {
    assert!(document::load_bytes(vec![]).is_err());
    let error = document::load_bytes(vec![("broken.zip".into(), b"broken".to_vec())])
        .err()
        .unwrap();
    assert!(error.contains("Invalid ZIP"));
    assert!(document::load_bytes(vec![("top.gtl".into(), vec![0xff])]).is_err());
}

#[test]
#[ignore = "requires GERBER_TEST_ARCHIVE pointing to the private Cadence ZIP"]
fn private_archive_works_through_browser_byte_and_worker_paths() {
    let path = std::env::var_os("GERBER_TEST_ARCHIVE").expect("Set GERBER_TEST_ARCHIVE");
    let data = std::fs::read(&path).unwrap();
    let uploaded = document::load_bytes(vec![("private-cadence.zip".into(), data)]).unwrap();
    let decoded = transfer::decode(&transfer::encode(&uploaded).unwrap()).unwrap();
    assert_eq!(decoded.layers.len(), 15);
    assert_eq!(decoded.warning_count(), 0);
    assert!(decoded.bounds.size()[0] < 250.0 && decoded.bounds.size()[1] < 110.0);
    assert_eq!(decoded.primitives(), 102072);
}
