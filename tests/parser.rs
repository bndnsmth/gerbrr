use gerbrr::{
    document,
    geometry::Geometry,
    parser::{parse_excellon, parse_gerber},
};

fn gerber(body: &str) -> String {
    format!("%FSLAX36Y36*%\n%MOMM*%\n{body}\nM02*\n")
}

fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 0.015, "{a} != {b}");
}

// Sample tessellated triangles in command order, modeling the GPU's polarity mask.
fn covered(g: &Geometry, p: [f32; 2]) -> bool {
    let mut result = false;
    for batch in &g.batches {
        for triangle in g.indices[batch.indices.start as usize..batch.indices.end as usize]
            .as_chunks::<3>()
            .0
        {
            let points: Vec<_> = triangle
                .iter()
                .map(|i| g.vertices[*i as usize].position)
                .collect();
            let mut signs = [0.0; 3];
            for i in 0..3 {
                let a = points[i];
                let b = points[(i + 1) % 3];
                signs[i] = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
            }
            if signs.iter().all(|s| *s >= -1e-6) || signs.iter().all(|s| *s <= 1e-6) {
                result = batch.dark;
            }
        }
    }
    result
}

#[test]
fn demo_is_a_complete_board_without_warnings() {
    let doc = document::demo();
    assert_eq!(doc.layers.len(), 6);
    assert_eq!(doc.warning_count(), 0);
    assert!(doc.primitives() > 500);
    assert!(doc.triangles() > 5000);
    close(doc.bounds.size()[0], 80.3);
    close(doc.bounds.size()[1], 54.3);
}

#[test]
fn leading_zero_coordinates_and_modal_axes() {
    let source = "%FSLAX24Y24*%%MOMM*%%ADD10C,1*%D10*X10000Y20000D03*X30000D03*M02*";
    let p = parse_gerber(source).unwrap();
    close(p.geometry.bounds.min[0], 0.5);
    close(p.geometry.bounds.max[0], 3.5);
    close(p.geometry.bounds.min[1], 1.5);
    assert_eq!(p.geometry.features.len(), 2);
}

#[test]
fn trailing_zero_coordinates_and_negative_sign() {
    let source = "%FSTAX24Y24*%%MOMM*%%ADD10C,1*%D10*X-1Y2D03*M02*";
    let p = parse_gerber(source).unwrap();
    close(p.geometry.bounds.center()[0], -10.0);
    close(p.geometry.bounds.center()[1], 20.0);
}

#[test]
fn inch_coordinates_are_normalized() {
    let p = parse_gerber("%FSLAX24Y24*%%MOIN*%%ADD10C,0.1*%D10*X10000Y10000D03*M02*").unwrap();
    close(p.geometry.bounds.center()[0], 25.4);
    close(p.geometry.bounds.size()[0], 2.54);
}

#[test]
fn circle_aperture_hole_is_not_a_clear_operation() {
    let p = parse_gerber(&gerber("%ADD10C,2X1*%D10*X0Y0D03*")).unwrap();
    assert!(!covered(&p.geometry, [0.1, 0.1]));
    assert!(covered(&p.geometry, [0.8, 0.1]));
    let p = parse_gerber(&gerber(
        "%ADD10C,0.4*%%ADD11C,2X1*%D10*X0Y0D03*D11*X0Y0D03*",
    ))
    .unwrap();
    assert!(covered(&p.geometry, [0.05, 0.03]));
}

#[test]
fn polarity_clear_and_redraw() {
    let p = parse_gerber(&gerber(
        "%ADD10C,4*%%ADD11C,2*%%ADD12C,0.5*%D10*X0Y0D03*%LPC*%D11*X0Y0D03*%LPD*%D12*X0Y0D03*",
    ))
    .unwrap();
    assert!(covered(&p.geometry, [0.1, 0.05]));
    assert!(!covered(&p.geometry, [0.6, 0.1]));
    assert!(covered(&p.geometry, [1.5, 0.1]));
    assert_eq!(p.geometry.batches.len(), 3);
}

#[test]
fn rectangles_obrounds_polygons_and_rotation() {
    let p = parse_gerber(&gerber("%ADD10R,4X2*%D10*%LR90*%X0Y0D03*")).unwrap();
    close(p.geometry.bounds.size()[0], 2.0);
    close(p.geometry.bounds.size()[1], 4.0);
    let p = parse_gerber(&gerber("%ADD10O,4X2*%D10*X0Y0D03*")).unwrap();
    close(p.geometry.bounds.size()[0], 4.0);
    close(p.geometry.bounds.size()[1], 2.0);
    assert!(!covered(&p.geometry, [1.9, 0.9]));
    let p = parse_gerber(&gerber("%ADD10P,4X4X45*%D10*X0Y0D03*")).unwrap();
    close(p.geometry.bounds.size()[0], 2.0_f32.sqrt() * 2.0);
}

#[test]
fn rectangular_aperture_sweep() {
    let p = parse_gerber(&gerber("%ADD10R,2X1*%D10*X0Y0D02*X5.0Y5.0D01*")).unwrap();
    assert!(covered(&p.geometry, [3.1, 2.8]));
    assert!(!covered(&p.geometry, [2.0, 4.0]));
    close(p.geometry.bounds.min[0], -1.0);
    close(p.geometry.bounds.max[0], 6.0);
}

#[test]
fn region_holes_and_disconnected_contours() {
    let p = parse_gerber(&gerber("G36*X0Y0D02*X4.0Y0D01*X4.0Y4.0D01*X0Y4.0D01*X0Y0D01*X1.0Y1.0D02*X3.0Y1.0D01*X3.0Y3.0D01*X1.0Y3.0D01*X1.0Y1.0D01*G37*")).unwrap();
    assert!(covered(&p.geometry, [0.5, 0.7]));
    assert!(!covered(&p.geometry, [2.0, 2.1]));
    assert_eq!(p.geometry.features.len(), 1);
}

#[test]
fn multi_quadrant_arc_and_full_circle() {
    let p = parse_gerber(&gerber(
        "%ADD10C,0.2*%D10*G75*X2.0Y0D02*G03X0Y2.0I-2.0J0D01*",
    ))
    .unwrap();
    assert!(covered(&p.geometry, [1.414, 1.414]));
    assert!(!covered(&p.geometry, [1.0, 1.0]));
    let p = parse_gerber(&gerber(
        "%ADD10C,0.2*%D10*G75*X2.0Y0D02*G02X2.0Y0I-2.0J0D01*",
    ))
    .unwrap();
    close(p.geometry.bounds.size()[0], 4.2);
    close(p.geometry.bounds.size()[1], 4.2);
}

#[test]
fn single_quadrant_arc_center_disambiguation() {
    let p = parse_gerber(&gerber(
        "%ADD10C,0.2*%D10*G74*X2.0Y0D02*G03X0Y2.0I2.0J0D01*",
    ))
    .unwrap();
    assert!(covered(&p.geometry, [1.414, 1.414]));
}

#[test]
fn step_repeat_and_incremental_coordinates() {
    let p = parse_gerber(&gerber(
        "%ADD10C,1*%D10*%SRX3Y2I5J8*%X0Y0D03*%SR*%G91*X2.0Y3.0D03*X2.0D03*",
    ))
    .unwrap();
    assert_eq!(p.geometry.features.len(), 8);
    close(p.geometry.bounds.max[0], 10.5);
    close(p.geometry.features[7].bounds.center()[0], 4.0);
    close(p.geometry.features[7].bounds.center()[1], 3.0);
}

#[test]
fn macro_expressions_clear_hole_and_overlap_union() {
    let p = parse_gerber(&gerber(
        "%AMRING*$3=($1+$2)/2*1,1,$1,0,0*1,0,$2,0,0*%\n%ADD10RING,4X2*%D10*X0Y0D03*",
    ))
    .unwrap();
    assert!(!covered(&p.geometry, [0.2, 0.1]));
    assert!(covered(&p.geometry, [1.5, 0.1]));
    let p = parse_gerber(&gerber(
        "%AMDOUBLE*1,1,2,0,0*1,1,2,1,0*%\n%ADD10DOUBLE*%D10*X0Y0D03*",
    ))
    .unwrap();
    assert!(covered(&p.geometry, [0.5, 0.1]));
}

#[test]
fn macro_thermal_gaps_and_moire_holes() {
    let p = parse_gerber(&gerber(
        "%AMTHERMAL*7,0,0,4,2,0.4,0*%%ADD10THERMAL*%D10*X0Y0D03*",
    ))
    .unwrap();
    assert!(!covered(&p.geometry, [0.1, 0.1]));
    assert!(!covered(&p.geometry, [1.5, 0.1]));
    assert!(covered(&p.geometry, [1.1, 1.1]));
    let p = parse_gerber(&gerber(
        "%AMTARGET*6,0,0,4,0.2,0.2,2,0.1,0.5,0*%%ADD10TARGET*%D10*X0Y0D03*",
    ))
    .unwrap();
    assert!(!covered(&p.geometry, [0.8, 0.3]));
    assert!(covered(&p.geometry, [1.9, 0.1]));
}

#[test]
fn macro_outline_and_vector_line() {
    let p = parse_gerber(&gerber(
        "%AMVECTOR*20,1,1,0,0,4,0,0*4,1,3,0,1,2,1,1,3,0,1,0*%%ADD10VECTOR*%D10*X0Y0D03*",
    ))
    .unwrap();
    assert!(covered(&p.geometry, [2.0, 0.1]));
    assert!(covered(&p.geometry, [1.0, 1.5]));
    assert!(!covered(&p.geometry, [-0.2, 0.0]));
}

#[test]
fn malformed_inputs_fail_without_panics() {
    for source in [
        "",
        "not a gerber",
        "%FSLAX24Y24*",
        "%FSLAX24Y24*%X0Y0D03*M02*",
        "%MOMM*%%ADD10C,nan*%",
        "%FSLAX24Y24*%%SRX99999Y99999I1J1*%",
    ] {
        assert!(parse_gerber(source).is_err(), "{source}");
    }
    assert!(parse_excellon("M48\nMETRIC,TZ\nX1.0Y1.0\nM30").is_err());
}

#[test]
fn unsupported_geometry_is_reported() {
    let p = parse_gerber(&gerber("%IPNEG*%%ADD10C,1*%D10*X0Y0D03*")).unwrap();
    assert!(p.warnings.iter().any(|w| w.contains("IPNEG")));
    let p = parse_gerber(&gerber(
        "%ABD12*%%ADD10C,1*%D10*X0Y0D03*%AB*%D10*X1.0Y1.0D03*",
    ))
    .unwrap();
    assert_eq!(p.geometry.features.len(), 1);
    assert!(!p.warnings.is_empty());
}

#[test]
fn excellon_decimal_coordinates_and_slots() {
    let p =
        parse_excellon("M48\nMETRIC,TZ\nT1C0.8\n%\nT1\nX10.0Y20.0\nX15.0\nX0Y0G85X5.0Y0\nM30\n")
            .unwrap();
    assert_eq!(p.geometry.features.len(), 3);
    assert!(p.warnings.is_empty());
    close(p.geometry.bounds.max[0], 15.4);
    assert!(covered(&p.geometry, [2.5, 0.1]));
}

#[test]
fn excellon_zero_modes_and_explicit_format() {
    let p =
        parse_excellon("M48\n;FILE_FORMAT=2:4\nMETRIC,TZ\nT1C1\n%\nT1\nX10000Y20000\nM30").unwrap();
    close(p.geometry.bounds.center()[0], 1.0);
    close(p.geometry.bounds.center()[1], 2.0);
    let p = parse_excellon("M48\nINCH,LZ,00.0000\nT1C0.1\n%\nT1\nX01Y02\nM30").unwrap();
    close(p.geometry.bounds.center()[0], 25.4);
    close(p.geometry.bounds.center()[1], 50.8);
}

#[test]
fn excellon_repeat_holes_and_routes() {
    let p = parse_excellon(
        "M48\nMETRIC,TZ\nT1C1\n%\nT1\nX1.0Y1.0\nR3X2.0Y0\nG00X0Y0\nM15\nG01X5.0Y0\nM16\nM30",
    )
    .unwrap();
    assert_eq!(p.geometry.features.len(), 5);
    close(p.geometry.features[3].bounds.center()[0], 7.0);
    assert!(covered(&p.geometry, [2.0, 0.1]));
}

#[test]
fn spatial_inspection_prefers_small_feature() {
    let p = parse_gerber(&gerber("%ADD10C,4*%%ADD11C,1*%D10*X0Y0D03*D11*X0Y0D03*")).unwrap();
    assert_eq!(p.geometry.pick([0.1, 0.1], 0.0), Some(1));
    assert_eq!(p.geometry.pick([20.0, 20.0], 0.1), None);
}

#[test]
fn mixed_import_preserves_good_layers_and_errors() {
    let doc = document::from_sources(
        "Test".into(),
        vec![
            ("top.gtl".into(), gerber("%ADD10C,1*%D10*X0Y0D03*")),
            ("bad.gbr".into(), "broken".into()),
        ],
        false,
    )
    .unwrap();
    assert_eq!(doc.layers.len(), 1);
    assert_eq!(doc.errors.len(), 1);
}

#[test]
fn aperture_cache_tracks_transform_and_redefinition() {
    let p = parse_gerber(&gerber(
        "%ADD10R,4X2*%D10*X0Y0D03*%LR90*%X10.0Y0D03*%LS2*%X20.0Y0D03*%ADD10C,1*%X30.0Y0D03*",
    ))
    .unwrap();
    let f = &p.geometry.features;
    close(f[0].bounds.size()[0], 4.0);
    close(f[1].bounds.size()[0], 2.0);
    close(f[2].bounds.size()[1], 8.0);
    close(f[3].bounds.size()[0], 2.0);
}

#[test]
fn legacy_image_offset_preserves_modal_axes() {
    let p = parse_gerber(&gerber("%OFA10B20*%%ADD10C,1*%D10*X0D03*X2.0Y3.0D03*")).unwrap();
    close(p.geometry.features[0].bounds.center()[0], 10.0);
    close(p.geometry.features[0].bounds.center()[1], 20.0);
    close(p.geometry.features[1].bounds.center()[0], 12.0);
    close(p.geometry.features[1].bounds.center()[1], 23.0);
}

#[test]
fn unknown_macro_primitive_fails_instead_of_drawing_guessed_geometry() {
    let error = parse_gerber(&gerber("%AMUNKNOWN*99,1,2,3*%%ADD10UNKNOWN*%D10*X0Y0D03*"))
        .err()
        .expect("Unsupported macro must fail");
    assert!(error.contains("Unsupported aperture macro primitive 99"));
}
