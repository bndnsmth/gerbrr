//! Synthetic Cadence-style exports; no proprietary board data is embedded here.
use gerbrr::{
    document::{self, LayerKind},
    parser::{self, GerberFormatHint},
};

fn film(layers: &[&str], body: &str) -> String {
    let comments = layers
        .iter()
        .map(|layer| format!("G04 Layer: {layer}*\n"))
        .collect::<String>();
    format!(
        "G04 File Origin: Cadence Allegro*\n{comments}%FSLAX25Y25*MOIN*%\n%IR0*IPPOS*OFA0.00000B0.00000*MIA0B0*SFA1.00000B1.00000*%\n%ADD10C,.02*%\nD10*\n{body}\nM02*\n"
    )
}

fn pad(layers: &[&str]) -> String {
    film(layers, "X100000Y100000D03*")
}
fn close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.002, "{actual} != {expected}");
}
fn inch_hint(integer: usize) -> GerberFormatHint {
    GerberFormatHint {
        integer,
        decimal: 5,
        unit_mm: 25.4,
    }
}

#[test]
fn cadence_identity_headers_are_not_unsupported_geometry() {
    let parsed = parser::parse_gerber(&pad(&["ETCH/TOP"])).unwrap();
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
    close(parsed.geometry.bounds.center()[0], 25.4);
}

#[test]
fn non_identity_image_transforms_still_produce_warnings() {
    for transform in ["IR90", "MIA1B0", "SFA2.0B1.0"] {
        let source = pad(&[]).replace("%ADD10", &format!("%{transform}*%\n%ADD10"));
        let parsed = parser::parse_gerber(&source).unwrap();
        assert!(
            parsed.warnings.iter().any(|w| w.contains("non-identity")),
            "{transform}"
        );
    }
}

#[test]
fn lz_drills_keep_the_same_scale_when_coordinate_precision_varies() {
    let source = "M48\nINCH,LZ\nT01C.008\n%\nG90\nT01\nX05212Y00559\nX051849Y00544\nX0639191Y0128409\nX-00639191Y+00128409\nM30\n";
    let parsed = parser::parse_excellon_with_hint(source, Some(inch_hint(2))).unwrap();
    assert!(parsed.warnings.is_empty());
    for (feature, expected) in parsed.geometry.features.iter().zip([
        [5.212 * 25.4, 0.559 * 25.4],
        [5.1849 * 25.4, 0.544 * 25.4],
        [6.39191 * 25.4, 1.28409 * 25.4],
        [-0.639191 * 25.4, 0.128409 * 25.4],
    ]) {
        let center = feature.bounds.center();
        close(center[0], expected[0]);
        close(center[1], expected[1]);
    }
    let standalone = parser::parse_excellon(source).unwrap();
    assert_eq!(standalone.warnings.len(), 1);
    assert!(standalone.warnings[0].contains("variable decimal precision"));
    close(
        standalone.geometry.features[2].bounds.center()[0],
        6.39191 * 25.4,
    );
}

#[test]
fn explicit_drill_formats_override_context_and_tz_precision_is_not_guessed() {
    let explicit = parser::parse_excellon_with_hint(
        "M48\n;FILE_FORMAT=3:3\nINCH,LZ\nT1C.01\n%\nT1\nX0012Y0001\nM30",
        Some(inch_hint(2)),
    )
    .unwrap();
    close(explicit.geometry.bounds.center()[0], 1.2 * 25.4);
    assert!(explicit.warnings.is_empty());
    let tz = parser::parse_excellon_with_hint(
        "M48\nINCH,TZ\nT1C.01\n%\nT1\nX10000Y20000\nM30",
        Some(inch_hint(2)),
    )
    .unwrap();
    close(tz.geometry.bounds.center()[0], 25.4);
    assert!(
        !tz.warnings.is_empty(),
        "A companion film must not silently override drill decimal precision"
    );
    let metric = parser::parse_excellon_with_hint(
        "M48\nMETRIC,LZ\nT1C1.0\n%\nT1\nX0012Y0001\nM30",
        Some(inch_hint(2)),
    )
    .unwrap();
    close(metric.geometry.bounds.center()[0], 1.2);
    assert!(
        !metric.warnings.is_empty(),
        "Hints with incompatible units must be ignored"
    );
}

#[test]
fn format_hint_reads_combined_parameter_blocks_without_panicking_on_bad_formats() {
    let hint = parser::gerber_format_hint(&pad(&[])).unwrap();
    assert_eq!((hint.integer, hint.decimal), (2, 5));
    close(hint.unit_mm, 25.4);
    for source in [
        "%FSX-1Y-1*MOIN*%",
        "%FSX25Y26*MOIN*%",
        "%FSX99Y99*%",
        "plain text",
    ] {
        assert!(parser::gerber_format_hint(source).is_none(), "{source}");
    }
}

#[test]
fn cadence_names_classify_copper_masks_paste_silk_and_drawings() {
    for (name, kind, visible) in [
        ("02_TOP.art", LayerKind::Copper, true),
        ("03_L2_GND.art", LayerKind::InnerCopper, false),
        ("04_L3_PWR.art", LayerKind::InnerCopper, false),
        ("05_BOTTOM.art", LayerKind::BottomCopper, true),
        ("06_sst.art", LayerKind::Silk, true),
        ("07_ssb.art", LayerKind::BottomSilk, false),
        ("08_smt.art", LayerKind::Mask, false),
        ("09_smb.art", LayerKind::Mask, false),
        ("10_spt.art", LayerKind::Paste, false),
        ("11_spb.art", LayerKind::Paste, false),
        ("12_assy_t.art", LayerKind::Assembly, false),
        ("13_assy_b.art", LayerKind::Assembly, false),
        ("01_drilldwg.art", LayerKind::Drawing, false),
    ] {
        let doc =
            document::from_sources("Test".into(), vec![(name.into(), pad(&[]))], false).unwrap();
        assert_eq!(doc.layers[0].kind, kind, "{name}");
        assert_eq!(doc.layers[0].visible, visible, "{name}");
    }
}

#[test]
fn legacy_metadata_works_even_when_files_are_renamed_and_x2_takes_precedence() {
    for (layers, kind) in [
        (
            vec!["DRAWING FORMAT/FAB_FILM", "ETCH/TOP"],
            LayerKind::Copper,
        ),
        (vec!["ETCH/L3_PWR"], LayerKind::InnerCopper),
        (vec!["BOARD GEOMETRY/SOLDERMASK_TOP"], LayerKind::Mask),
        (vec!["PIN/PASTEMASK_TOP"], LayerKind::Paste),
        (vec!["REF DES/SILKSCREEN_BOTTOM"], LayerKind::BottomSilk),
        (
            vec![
                "BOARD GEOMETRY/SILKSCREEN_TOP",
                "PACKAGE GEOMETRY/ASSEMBLY_TOP",
            ],
            LayerKind::Assembly,
        ),
        (
            vec!["BOARD GEOMETRY/OUTLINE", "MANUFACTURING/NCLEGEND-1-4"],
            LayerKind::Drawing,
        ),
    ] {
        let doc = document::from_sources(
            "Test".into(),
            vec![("renamed.art".into(), pad(&layers))],
            false,
        )
        .unwrap();
        assert_eq!(doc.layers[0].kind, kind);
    }
    let source = pad(&["ETCH/TOP"]).replace("%ADD10", "%TF.FileFunction,Soldermask,Top*%\n%ADD10");
    let doc =
        document::from_sources("Test".into(), vec![("02_TOP.art".into(), source)], false).unwrap();
    assert_eq!(doc.layers[0].kind, LayerKind::Mask);
}

#[test]
fn route_and_drill_files_share_integer_width_only_with_same_directory_films() {
    let drill = "M48\nINCH,LZ\nT1C.02\n%\nT1\nX05212Y00559\nM30";
    let route = "M48\nINCH,LZ\nT1C.02\n%\nG90\nF1\nS10000\nT1\nM16\nG00X0295236Y0121339\nG40\nM15\nG01X0296418Y0121339\nM16\nM30";
    let doc = document::from_sources(
        "Test".into(),
        vec![
            ("Gerbers/02_TOP.art".into(), pad(&["ETCH/TOP"])),
            ("Gerbers/holes.drl".into(), drill.into()),
            ("Gerbers/slots.rou".into(), route.into()),
        ],
        false,
    )
    .unwrap();
    assert_eq!(doc.layers.len(), 3);
    assert_eq!(doc.warning_count(), 0);
    let slots = doc
        .layers
        .iter()
        .find(|l| l.name.ends_with(".rou"))
        .unwrap();
    assert_eq!(slots.kind, LayerKind::Drill);
    assert_eq!(slots.geometry.features.len(), 1);
    close(
        slots.geometry.bounds.center()[0],
        (2.95236 + 2.96418) * 0.5 * 25.4,
    );
    assert!(document::supported_path(std::path::Path::new("slots.ROU")));
    let other_directory = document::from_sources(
        "Test".into(),
        vec![
            ("board-a/02_TOP.art".into(), pad(&[])),
            ("board-b/holes.drl".into(), drill.into()),
        ],
        false,
    )
    .unwrap();
    assert_eq!(other_directory.warning_count(), 1);
}

#[test]
fn contradictory_companion_formats_do_not_silence_ambiguity_warnings() {
    let source = pad(&[]);
    let doc = document::from_sources(
        "Test".into(),
        vec![
            ("top.gtl".into(), source.clone()),
            ("bottom.gbl".into(), source.replace("X25Y25", "X35Y35")),
            (
                "holes.drl".into(),
                "M48\nINCH,LZ\nT1C.01\n%\nT1\nX0012Y0001\nM30".into(),
            ),
        ],
        false,
    )
    .unwrap();
    assert_eq!(doc.warning_count(), 1);
}

#[test]
fn hidden_fabrication_sheets_do_not_inflate_board_or_default_fit_bounds() {
    let mut doc = document::from_sources(
        "Test".into(),
        vec![
            ("02_TOP.art".into(), pad(&["ETCH/TOP"])),
            (
                "01_drilldwg.art".into(),
                film(&["MANUFACTURING/NCLEGEND-1-4"], "X2000000Y1500000D03*"),
            ),
        ],
        false,
    )
    .unwrap();
    assert!(doc.bounds.size()[0] < 1.0);
    assert!(doc.visible_bounds().size()[0] < 1.0);
    doc.layers
        .iter_mut()
        .find(|l| l.kind == LayerKind::Drawing)
        .unwrap()
        .visible = true;
    assert!(
        doc.visible_bounds().size()[0] > 400.0,
        "Fitting enabled drawings must still show their full extent"
    );
}

#[test]
#[ignore = "requires GERBER_TEST_ARCHIVE pointing to the private Cadence ZIP"]
fn actual_cadence_archive_smoke_test() {
    let path = std::env::var_os("GERBER_TEST_ARCHIVE").expect("Set GERBER_TEST_ARCHIVE");
    let doc = document::load(vec![path.into()]).unwrap();
    assert_eq!(doc.layers.len(), 15);
    assert_eq!(doc.warning_count(), 0);
    assert!(doc.bounds.size()[0] < 250.0 && doc.bounds.size()[1] < 110.0);
    assert_eq!(
        doc.layers
            .iter()
            .filter(|l| l.kind == LayerKind::Copper)
            .count(),
        1
    );
    assert_eq!(
        doc.layers
            .iter()
            .filter(|l| l.kind == LayerKind::InnerCopper)
            .count(),
        2
    );
    assert_eq!(
        doc.layers
            .iter()
            .filter(|l| l.kind == LayerKind::Drill)
            .count(),
        2
    );
    assert!(doc.layers.iter().all(|l| l.kind != LayerKind::Other));
    assert!(
        doc.layers
            .iter()
            .filter(|l| matches!(l.kind, LayerKind::Drawing | LayerKind::Assembly))
            .all(|l| !l.visible)
    );
    let drills = doc
        .layers
        .iter()
        .find(|l| l.name.ends_with(".drl"))
        .unwrap();
    assert_eq!(drills.geometry.features.len(), 1359);
    close(drills.geometry.features[0].bounds.center()[0], 5.212 * 25.4);
    close(drills.geometry.features[0].bounds.center()[1], 0.559 * 25.4);
    assert_eq!(
        doc.layers
            .iter()
            .find(|l| l.name.ends_with(".rou"))
            .unwrap()
            .geometry
            .features
            .len(),
        28
    );
}
