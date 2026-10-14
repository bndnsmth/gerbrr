use gerbrr::{
    document::{self, LayerKind},
    geometry::MeshBuilder,
    parser,
    snapping::{SnapKind, SnapMode},
    transfer,
};

fn gerber(body: &str) -> gerbrr::geometry::Geometry {
    parser::parse_gerber(&format!("%FSLAX36Y36*%%MOMM*%{body}M02*"))
        .unwrap()
        .geometry
}
fn close(a: [f32; 2], b: [f32; 2]) {
    assert!(
        (a[0] - b[0]).abs() < 0.0001 && (a[1] - b[1]).abs() < 0.0001,
        "{a:?} != {b:?}"
    );
}

#[test]
fn centers_use_declared_flash_coordinates_and_edges_use_exact_circle_radii() {
    let g = gerber("%ADD10C,1*%D10*X3000000Y4000000D03*");
    let hit = g.snap([3.03, 4.02], 0.08, SnapMode::Auto).unwrap();
    assert_eq!(hit.kind, SnapKind::Center);
    close(hit.point, [3.0, 4.0]);
    let hit = g.snap([3.49, 4.0], 0.03, SnapMode::Edges).unwrap();
    assert_eq!(hit.kind, SnapKind::Edge);
    close(hit.point, [3.5, 4.0]);
    let diagonal = std::f32::consts::FRAC_1_SQRT_2;
    let hit = g
        .snap(
            [3.0 + 0.51 * diagonal, 4.0 + 0.51 * diagonal],
            0.02,
            SnapMode::Edges,
        )
        .unwrap();
    close(hit.point, [3.0 + 0.5 * diagonal, 4.0 + 0.5 * diagonal]);
    assert!(g.snap([3.5, 4.0], 0.01, SnapMode::Centers).is_none());
}

#[test]
fn annular_pad_holes_have_real_inner_edges_and_a_declared_center() {
    let g = gerber("%ADD10C,2X0.8*%D10*X0Y0D03*");
    assert!(!g.covered([0.0, 0.0]));
    close(
        g.snap([0.01, 0.02], 0.05, SnapMode::Centers).unwrap().point,
        [0.0, 0.0],
    );
    close(
        g.snap([0.41, 0.0], 0.02, SnapMode::Edges).unwrap().point,
        [0.4, 0.0],
    );
}

#[test]
fn asymmetric_macro_origins_are_not_replaced_by_bounding_box_centers() {
    let g = gerber("%AMOFFSET*21,1,2,1,3,0,0*%%ADD10OFFSET*%D10*X0Y0D03*");
    close(g.features[0].bounds.center(), [3.0, 0.0]);
    close(
        g.snap([0.01, 0.0], 0.05, SnapMode::Centers).unwrap().point,
        [0.0, 0.0],
    );
    assert!(g.snap([3.0, 0.0], 0.05, SnapMode::Centers).is_none());
}

#[test]
fn regions_do_not_offer_box_centers_or_tessellation_diagonals() {
    let g = gerber(
        "%ADD10C,0.1*%D10*G36*X0Y0D02*X2000000Y0D01*X2000000Y2000000D01*X0Y2000000D01*X0Y0D01*G37*",
    );
    assert!(g.snap([1.0, 1.0], 0.05, SnapMode::Auto).is_none());
    close(
        g.snap([1.0, 2.01], 0.02, SnapMode::Edges).unwrap().point,
        [1.0, 2.0],
    );
    close(
        g.snap([2.005, 2.005], 0.02, SnapMode::Edges).unwrap().point,
        [2.0, 2.0],
    );
}

#[test]
fn rotated_and_mirrored_apertures_keep_real_contours_and_source_origins() {
    let g = gerber("%ADD10R,2X1*%%LMX*%%LR90*%D10*X3000000Y4000000D03*");
    close(
        g.snap([3.01, 4.0], 0.05, SnapMode::Centers).unwrap().point,
        [3.0, 4.0],
    );
    close(
        g.snap([3.51, 4.0], 0.02, SnapMode::Edges).unwrap().point,
        [3.5, 4.0],
    );
    // Corner of the rotated rectangle's bounds is a real corner; a bbox edge
    // of a rotated non-axis-aligned rectangle would not be a valid target.
    let g = gerber("%ADD10R,2X1*%%LR45*%D10*X0Y0D03*");
    assert!(g.snap([1.05, 0.0], 0.03, SnapMode::Edges).is_none());
}

#[test]
fn clears_remove_ghost_targets_and_repaint_restores_targets() {
    let erased = gerber("%ADD10C,2*%%ADD11R,6X6*%D10*X0Y0D03*%LPC*%D11*X0Y0D03*");
    assert!(erased.snap([0.0, 0.0], 0.05, SnapMode::Centers).is_none());
    assert!(erased.snap([1.0, 0.0], 0.05, SnapMode::Edges).is_none());
    let restored = gerber(
        "%ADD10C,2*%%ADD11R,6X6*%%ADD12C,0.5*%D10*X0Y0D03*%LPC*%D11*X0Y0D03*%LPD*%D12*X0Y0D03*",
    );
    close(
        restored
            .snap([0.24, 0.0], 0.02, SnapMode::Edges)
            .unwrap()
            .point,
        [0.25, 0.0],
    );
}

#[test]
fn clear_aperture_edges_exist_only_where_they_cut_painted_geometry() {
    let g = gerber("%ADD10C,4*%%ADD11C,1*%D10*X0Y0D03*%LPC*%D11*X0Y0D03*X5000000Y0D03*");
    close(
        g.snap([0.51, 0.0], 0.02, SnapMode::Edges).unwrap().point,
        [0.5, 0.0],
    );
    assert!(g.snap([5.0, 0.0], 0.05, SnapMode::Centers).is_none());
    assert!(g.snap([5.5, 0.0], 0.05, SnapMode::Edges).is_none());
}

#[test]
fn overlapping_features_do_not_offer_internal_overlap_seams() {
    let g = gerber("%ADD10C,2*%D10*X0Y0D03*X500000Y0D03*");
    assert!(g.snap([1.0, 0.0], 0.02, SnapMode::Edges).is_none());
    close(
        g.snap([1.49, 0.0], 0.02, SnapMode::Edges).unwrap().point,
        [1.5, 0.0],
    );
}

#[test]
fn round_strokes_snap_to_perimeters_and_endpoints_not_box_centers() {
    let g = gerber("%ADD10C,0.4*%D10*X0Y0D02*X3000000Y0D01*");
    let hit = g.snap([1.2, 0.19], 0.03, SnapMode::Edges).unwrap();
    close(hit.point, [1.2, 0.2]);
    close(
        g.snap([-0.19, 0.0], 0.03, SnapMode::Edges).unwrap().point,
        [-0.2, 0.0],
    );
    let hit = g.snap([0.01, 0.01], 0.03, SnapMode::Centers).unwrap();
    assert_eq!(hit.kind, SnapKind::Endpoint);
    close(hit.point, [0.0, 0.0]);
    assert!(g.snap([1.5, 0.0], 0.03, SnapMode::Centers).is_none());
    let mut mesh = MeshBuilder::default();
    mesh.stroke(&[[0.0, 0.0], [2.0, 0.0], [2.0, 2.0]], 0.4, true, "Trace")
        .unwrap();
    let g = mesh.finish();
    assert!(
        g.snap([1.9, 0.2], 0.03, SnapMode::Edges).is_none(),
        "An internal join must not become a clearance edge"
    );
}

#[test]
fn drills_repeats_and_straight_slots_have_exact_source_centers() {
    let g = parser::parse_excellon(
        "M48\nMETRIC\nT1C1.0\n%\nT1\nX10.0Y5.0\nR02X2.54\nX20.0Y5.0G85X22.0Y5.0\nM30",
    )
    .unwrap()
    .geometry;
    for p in [[10.0, 5.0], [12.54, 5.0], [15.08, 5.0], [21.0, 5.0]] {
        let hit = g
            .snap([p[0] + 0.01, p[1]], 0.03, SnapMode::Centers)
            .unwrap();
        assert_eq!(hit.kind, SnapKind::Center);
        close(hit.point, p);
    }
    close(
        g.snap([21.0, 5.49], 0.02, SnapMode::Edges).unwrap().point,
        [21.0, 5.5],
    );
    assert_eq!(
        g.snap_shapes.len(),
        2,
        "Repeated round holes share one source shape"
    );
}

#[test]
fn step_repeated_pads_share_shapes_without_losing_their_individual_origins() {
    let g = gerber("%ADD10C,1*%%SRX3Y2I2.54J4*%D10*X1000000Y2000000D03*%SR*%");
    assert_eq!(g.snap_shapes.len(), 1);
    for p in [[1.0, 2.0], [3.54, 2.0], [6.08, 6.0]] {
        close(
            g.snap([p[0] + 0.02, p[1]], 0.03, SnapMode::Centers)
                .unwrap()
                .point,
            p,
        );
    }
}

#[test]
fn only_visible_nontransparent_layers_snap_and_selected_layer_breaks_ties() {
    let source = "%FSLAX36Y36*%%MOMM*%%ADD10C,1*%D10*X0Y0D03*M02*";
    let mut doc = document::from_sources(
        "Test".into(),
        vec![
            ("a.gtl".into(), source.into()),
            ("b.gbl".into(), source.into()),
        ],
        false,
    )
    .unwrap();
    let top = doc
        .layers
        .iter()
        .position(|l| l.kind == LayerKind::Copper)
        .unwrap();
    let bottom = doc
        .layers
        .iter()
        .position(|l| l.kind == LayerKind::BottomCopper)
        .unwrap();
    assert_eq!(
        doc.snap([0.01, 0.0], 0.05, SnapMode::Auto, top)
            .unwrap()
            .layer,
        top
    );
    doc.layers[top].visible = false;
    assert_eq!(
        doc.snap([0.01, 0.0], 0.05, SnapMode::Auto, top)
            .unwrap()
            .layer,
        bottom
    );
    doc.layers[bottom].opacity = 0.0;
    assert!(doc.snap([0.01, 0.0], 0.05, SnapMode::Auto, top).is_none());
}

#[test]
fn worker_transfer_rebuilds_snap_indexes_and_rejects_invalid_target_metadata() {
    let mut doc = document::from_sources(
        "Test".into(),
        vec![(
            "top.gtl".into(),
            "%FSLAX36Y36*%%MOMM*%%ADD10C,1*%D10*X0Y0D03*M02*".into(),
        )],
        false,
    )
    .unwrap();
    let decoded = transfer::decode(&transfer::encode(&doc).unwrap()).unwrap();
    assert_eq!(
        doc.snap([0.01, 0.0], 0.05, SnapMode::Auto, 0),
        decoded.snap([0.01, 0.0], 0.05, SnapMode::Auto, 0)
    );
    let g = std::sync::Arc::get_mut(&mut doc.layers[0].geometry).unwrap();
    g.features[0].snap.center = Some([f32::NAN, 0.0]);
    assert!(transfer::decode(&transfer::encode(&doc).unwrap()).is_err());
}

#[test]
fn invalid_queries_and_empty_geometry_have_no_targets() {
    let g = gerbrr::geometry::Geometry::default();
    for (p, r) in [
        ([0.0, 0.0], 1.0),
        ([f32::NAN, 0.0], 1.0),
        ([0.0, 0.0], f32::INFINITY),
        ([0.0, 0.0], -1.0),
    ] {
        assert!(g.snap(p, r, SnapMode::Auto).is_none());
    }
}

#[test]
fn large_contour_edge_indexes_survive_worker_transfer() {
    let mut source = String::from("%FSLAX36Y36*%%MOMM*%G36*");
    for i in 0..128 {
        let angle = i as f32 * std::f32::consts::TAU / 128.0;
        source.push_str(&format!(
            "X{}Y{}D0{}*",
            (10_000_000.0 * angle.cos()).round() as i32,
            (10_000_000.0 * angle.sin()).round() as i32,
            if i == 0 { 2 } else { 1 }
        ));
    }
    source.push_str("G37*M02*");
    let doc =
        document::from_sources("Test".into(), vec![("top.gtl".into(), source)], false).unwrap();
    let decoded = transfer::decode(&transfer::encode(&doc).unwrap()).unwrap();
    let a = doc.snap([10.01, 0.0], 0.03, SnapMode::Edges, 0).unwrap();
    let b = decoded
        .snap([10.01, 0.0], 0.03, SnapMode::Edges, 0)
        .unwrap();
    close(a.hit.point, [10.0, 0.0]);
    assert_eq!(a, b);
}
