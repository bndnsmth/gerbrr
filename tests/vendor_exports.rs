//! Small, original reproductions of quirks found in the public hardware corpus.
//! These run offline; upstream board artwork remains in the opt-in corpus cache.
use gerbrr::{
    document::{self, LayerKind},
    parser::{self, parse_excellon, parse_gerber},
};

fn film(function: &str) -> String {
    format!(
        "G04 #@! TF.FileFunction,{function}*\n%FSLAX25Y25*MOIN*%\n%ADD10C,.01*%\nD10*X100000Y100000D03*M02*"
    )
}

fn nc_settings(decimal: usize) -> String {
    format!(
        "INTEGER-PLACES 2\nDECIMAL-PLACES {decimal}\nOUTPUT-UNITS ENGLISH\nSUPPRESS-LEAD-ZEROES NO\nSUPPRESS-TRAIL-ZEROES NO\n"
    )
}

fn close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.002, "{actual} != {expected}");
}

#[test]
fn kicad_empty_npth_export_is_valid_but_arbitrary_text_is_not() {
    let source =
        "M48\n; FORMAT={-:-/ absolute / inch / decimal}\nFMAT,2\nINCH\n%\nG90\nG05\nT0\nM30\n";
    let parsed = parse_excellon(source).unwrap();
    assert!(parsed.geometry.features.is_empty());
    assert!(parsed.warnings.is_empty());
    assert_eq!(parsed.apertures, 0);
    for invalid in [
        "",
        "M48",
        "M48\ninvalid\nM30",
        "M30",
        "M48\nINCH\nX1.0Y2.0\nM30",
    ] {
        assert!(parse_excellon(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn comment_carried_x2_attributes_classify_renamed_layers() {
    for (function, kind) in [
        ("Copper,L2,Inr", LayerKind::InnerCopper),
        ("Legend,Bot", LayerKind::BottomSilk),
        ("Other,Drawing", LayerKind::Drawing),
    ] {
        let doc = document::from_sources(
            "Test".into(),
            vec![("renamed.gbr".into(), film(function))],
            false,
        )
        .unwrap();
        assert_eq!(doc.layers[0].kind, kind);
        assert_eq!(doc.warning_count(), 0);
    }
}

#[test]
fn altium_inner_copper_mechanical_and_pad_master_extensions_are_retained() {
    let source =
        film("Copper,L1,Top").replace("G04 #@! TF.FileFunction,Copper,L1,Top*", "G04 Example*");
    for (name, kind, visible) in [
        ("board.G1", LayerKind::InnerCopper, false),
        ("board.G10", LayerKind::InnerCopper, false),
        ("board.GM14", LayerKind::Drawing, false),
        ("board.GM15", LayerKind::Drawing, false),
        ("board.GPB", LayerKind::Drawing, false),
        ("board.GPT", LayerKind::Drawing, false),
    ] {
        assert!(document::supported_path(std::path::Path::new(name)));
        let doc = document::from_sources("Test".into(), vec![(name.into(), source.clone())], false)
            .unwrap();
        assert_eq!(doc.layers[0].kind, kind, "{name}");
        assert_eq!(doc.layers[0].visible, visible, "{name}");
    }
}

#[test]
fn txt_reports_are_not_layers_but_txt_drills_and_corrupt_artwork_are_not_hidden() {
    let doc = document::load_bytes(vec![
        ("board/board.gtl".into(), film("Copper,L1,Top").into_bytes()),
        (
            "board/readme.txt".into(),
            b"Gerber files and manufacturing instructions".to_vec(),
        ),
        (
            "board/Pick Place.txt".into(),
            b"Designator Center-X Center-Y Layer".to_vec(),
        ),
        (
            "board/drills.TXT".into(),
            b"\xef\xbb\xbfM48\nINCH\nT1C.01\n%\nT1\nX1.0Y1.0\nM30".to_vec(),
        ),
        (
            "board/broken.TXT".into(),
            b"M48\nINCH\nT1Cnot-a-number\nM30".to_vec(),
        ),
    ])
    .unwrap();
    assert_eq!(doc.layers.len(), 2);
    assert_eq!(doc.warning_count(), 1);
    assert!(doc.errors[0].contains("broken.TXT"));
    assert!(doc.layers.iter().any(|l| l.kind == LayerKind::Drill));
}

#[test]
fn cadence_nc_settings_fix_full_width_drills_without_guessing_from_films() {
    let drill = "M48\nINCH\nT01C.01\n%\nG90\nT01\nX0127343Y0168938\nM30";
    let doc = document::from_sources(
        "Test".into(),
        vec![
            ("board/nc_param.txt".into(), nc_settings(5)),
            ("board/holes.drl".into(), drill.into()),
            ("board/top.gtl".into(), film("Copper,L1,Top")),
        ],
        false,
    )
    .unwrap();
    assert_eq!(doc.layers.len(), 2);
    assert_eq!(doc.warning_count(), 0);
    let drills = doc
        .layers
        .iter()
        .find(|l| l.kind == LayerKind::Drill)
        .unwrap();
    close(drills.geometry.bounds.center()[0], 1.27343 * 25.4);
    close(drills.geometry.bounds.center()[1], 1.68938 * 25.4);
    assert_eq!(
        parse_excellon(drill).unwrap().warnings.len(),
        1,
        "No sidecar means the original ambiguity must still be reported"
    );
}

#[test]
fn explicit_drill_format_wins_and_nc_settings_do_not_cross_directories() {
    let drill = "M48\nINCH\nT1C.01\n%\nT1\nX100000Y200000\nM30";
    let doc = document::from_sources(
        "Test".into(),
        vec![
            ("a/nc_param.txt".into(), nc_settings(5)),
            ("a/holes.drl".into(), drill.into()),
            ("b/holes.drl".into(), drill.into()),
            ("c/nc_param.txt".into(), nc_settings(5)),
            (
                "c/holes.drl".into(),
                drill.replace("M48", "M48\n;FILE_FORMAT=2:4"),
            ),
        ],
        false,
    )
    .unwrap();
    assert_eq!(doc.warning_count(), 1);
    for layer in &doc.layers {
        close(
            layer.geometry.bounds.center()[0],
            if layer.name.starts_with("a/") {
                25.4
            } else {
                254.0
            },
        );
    }
}

#[test]
fn invalid_nc_settings_produce_diagnostics_not_a_silent_format_override() {
    for settings in [
        nc_settings(99),
        nc_settings(5)
            .replace("SUPPRESS-LEAD-ZEROES NO", "SUPPRESS-LEAD-ZEROES YES")
            .replace("SUPPRESS-TRAIL-ZEROES NO", "SUPPRESS-TRAIL-ZEROES YES"),
    ] {
        assert!(parser::cadence_nc_format(&settings).is_none());
        let doc = document::from_sources(
            "Test".into(),
            vec![
                ("nc_param.txt".into(), settings),
                ("top.gtl".into(), film("Copper,L1,Top")),
            ],
            false,
        )
        .unwrap();
        assert_eq!(doc.warning_count(), 1);
        assert!(doc.errors[0].contains("Invalid Cadence NC export settings"));
    }
}

#[test]
fn cadence_comment_tool_table_and_m00_blocks_preserve_all_holes_and_repeats() {
    let source = ";   Holesize 1. = 8.000000 Tolerance = +0/-0 PLATED MILS Quantity = 3\n;   Holesize 2. = 125.000000 Tolerance = +0/-0 NON_PLATED MILS Quantity = 1\n%\nG90\nX0129522Y0132875\nR02X0002000\nM00\nX0100000Y0200000\nM30";
    let hint = parser::cadence_nc_format(&nc_settings(5)).unwrap();
    let parsed = parser::parse_excellon_with_hints(source, None, Some(hint)).unwrap();
    assert!(parsed.warnings.is_empty());
    assert_eq!(parsed.apertures, 2);
    assert_eq!(parsed.geometry.features.len(), 4);
    close(
        parsed.geometry.features[0].bounds.center()[0],
        1.29522 * 25.4,
    );
    close(parsed.geometry.features[0].bounds.size()[0], 0.008 * 25.4);
    close(
        parsed.geometry.features[2].bounds.center()[0],
        (1.29522 + 0.02 + 0.02) * 25.4,
    );
    close(parsed.geometry.features[3].bounds.size()[0], 0.125 * 25.4);
    assert!(
        parser::parse_excellon_with_hints(
            &source.replace("Holesize 2.", "Holesize 3."),
            None,
            Some(hint)
        )
        .is_err()
    );
    assert!(
        parser::parse_excellon_with_hints(&source.replace("M30", "M00\nM30"), None, Some(hint))
            .is_err()
    );
}

#[test]
fn ordinary_m00_is_a_pause_not_end_of_file() {
    let p = parse_excellon("M48\nINCH\nT1C.01\nT2C.02\n%\nT1\nX1.0Y2.0\nM00\nT2\nX3.0Y4.0\nM30")
        .unwrap();
    assert_eq!(p.geometry.features.len(), 2);
    assert!(p.warnings.is_empty());
}

#[test]
fn windows_utf8_boms_do_not_change_gerber_or_drill_geometry() {
    let source = film("Copper,L1,Top");
    let plain = parse_gerber(&source).unwrap();
    let bom = parse_gerber(&format!("\u{feff}{source}")).unwrap();
    assert_eq!(plain.geometry.bounds, bom.geometry.bounds);
    assert_eq!(plain.file_function, bom.file_function);
    assert_eq!(plain.warnings, bom.warnings);
}

#[test]
fn archive_and_directory_names_do_not_override_layer_classification() {
    let source =
        film("Copper,L1,Top").replace("G04 #@! TF.FileFunction,Copper,L1,Top*", "G04 Example*");
    let doc = document::from_sources(
        "Test".into(),
        vec![("assembly-mask-bottom.zip/soldermask/top.gtl".into(), source)],
        false,
    )
    .unwrap();
    assert_eq!(doc.layers[0].kind, LayerKind::Copper);
}

#[test]
fn zip_imports_do_not_share_nc_settings_across_archives() {
    use std::io::{Cursor, Write};
    let archive = |decimal| {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, source) in [
            ("MFG/nc_param.txt", nc_settings(decimal)),
            (
                "MFG/holes.drl",
                "M48\nINCH\nT1C.01\n%\nT1\nX100000Y200000\nM30".into(),
            ),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(source.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    };
    let doc = document::load_bytes(vec![
        ("five.zip".into(), archive(5)),
        ("four.zip".into(), archive(4)),
    ])
    .unwrap();
    assert_eq!(doc.layers.len(), 2);
    assert_eq!(doc.warning_count(), 0);
    for layer in &doc.layers {
        close(
            layer.geometry.bounds.center()[0],
            if layer.name.starts_with("five.zip/") {
                25.4
            } else {
                254.0
            },
        );
    }
}

#[test]
fn nc_settings_keep_leading_and_trailing_zero_suppression_distinct() {
    for (settings, coordinate) in [
        (
            nc_settings(5).replace("SUPPRESS-TRAIL-ZEROES NO", "SUPPRESS-TRAIL-ZEROES YES"),
            "X0123Y0100",
        ),
        (
            nc_settings(5).replace("SUPPRESS-LEAD-ZEROES NO", "SUPPRESS-LEAD-ZEROES YES"),
            "X123000Y100000",
        ),
    ] {
        let hint = parser::cadence_nc_format(&settings).unwrap();
        let source = format!("M48\nINCH\nT1C.01\n%\nT1\n{coordinate}\nM30");
        let parsed = parser::parse_excellon_with_hints(&source, None, Some(hint)).unwrap();
        assert!(parsed.warnings.is_empty());
        close(parsed.geometry.bounds.center()[0], 1.23 * 25.4);
        close(parsed.geometry.bounds.center()[1], 25.4);
    }
}

#[test]
fn drill_units_and_inline_precision_override_incompatible_nc_settings() {
    let hint = parser::cadence_nc_format(&nc_settings(5)).unwrap();
    let parsed = parser::parse_excellon_with_hints(
        "M48\n;FILE_FORMAT=3:3\nMETRIC,TZ\nT1C1.0\n%\nT1\nX1000Y2000\nM30",
        None,
        Some(hint),
    )
    .unwrap();
    close(parsed.geometry.bounds.center()[0], 1.0);
    close(parsed.geometry.bounds.center()[1], 2.0);
    close(parsed.geometry.bounds.size()[0], 1.0);
    assert_eq!(parsed.warnings.len(), 1);
    assert!(parsed.warnings[0].contains("disagree with drill units"));
}

#[test]
fn contradictory_nc_sidecars_cannot_be_resurrected_by_a_later_matching_copy() {
    let doc = document::from_sources(
        "Test".into(),
        vec![
            ("nc_param.txt".into(), nc_settings(5)),
            ("nc_param.txt".into(), nc_settings(4)),
            ("nc_param.txt".into(), nc_settings(5)),
            (
                "holes.drl".into(),
                "M48\nINCH\nT1C.01\n%\nT1\nX10000Y20000\nM30".into(),
            ),
        ],
        false,
    )
    .unwrap();
    assert_eq!(doc.layers.len(), 1);
    assert_eq!(doc.warning_count(), 2);
    assert!(doc.errors[0].contains("Conflicting NC export settings"));
    assert!(doc.layers[0].warnings[0].contains("format not declared"));
}
