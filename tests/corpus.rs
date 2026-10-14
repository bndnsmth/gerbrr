#![cfg(not(target_arch = "wasm32"))]
//! Opt-in, checksum-pinned upstream artwork. Normal tests never access the network.
use gerbrr::{
    document::{self, Document, LayerKind},
    transfer,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Write},
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
struct Manifest {
    schema: u32,
    samples: Vec<Sample>,
}
#[derive(Deserialize)]
struct Sample {
    id: String,
    toolchain: String,
    name: String,
    repo: String,
    commit: String,
    license: String,
    generation: String,
    expected: Expected,
    files: Vec<File>,
    #[serde(rename = "licenseNote", default)]
    license_note: Option<String>,
}
#[derive(Deserialize)]
struct File {
    source: String,
    role: String,
    bytes: usize,
    sha256: String,
}
#[derive(Deserialize)]
struct Expected {
    layers: usize,
    primitives: usize,
    kinds: BTreeMap<LayerKind, usize>,
    bounds_mm: [f32; 2],
    warnings: Vec<String>,
    #[serde(default)]
    drill: Option<DrillProbe>,
}
#[derive(Deserialize)]
struct DrillProbe {
    suffix: String,
    hits: usize,
    tools: usize,
    first_center: [f32; 2],
    first_diameter: f32,
    #[serde(default)]
    tool_hits: Vec<usize>,
}

fn manifest() -> Manifest {
    serde_json::from_str(include_str!("fixtures/corpus.json"))
        .expect("Valid public corpus manifest")
}

#[test]
fn public_corpus_manifest_has_pinned_provenance_and_multiple_examples_per_toolchain() {
    let manifest = manifest();
    assert_eq!(manifest.schema, 1);
    let mut ids = BTreeSet::new();
    let mut vendors = BTreeMap::new();
    for sample in manifest.samples {
        assert!(ids.insert(sample.id));
        *vendors.entry(sample.toolchain).or_insert(0) += 1;
        assert!(!sample.name.is_empty() && !sample.generation.is_empty());
        assert_eq!(sample.repo.split('/').count(), 2);
        assert_eq!(sample.commit.len(), 40);
        assert!(sample.commit.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(!sample.license.is_empty());
        if sample.license.starts_with("LicenseRef-") {
            assert!(sample.license_note.is_some());
        }
        assert_eq!(
            sample.expected.kinds.values().sum::<usize>(),
            sample.expected.layers
        );
        assert!(sample.expected.primitives > 0);
        assert!(sample.files.iter().any(|f| f.role == "input"));
        assert!(sample.files.iter().any(|f| f.role == "provenance"));
        let mut filenames = BTreeSet::new();
        for file in sample.files {
            assert!(matches!(file.role.as_str(), "input" | "provenance"));
            assert!(
                file.source
                    .split('/')
                    .all(|p| !p.is_empty() && p != ".." && p != ".")
            );
            assert!(!Path::new(&file.source).is_absolute());
            assert!(filenames.insert((file.role, filename(&file.source).to_owned())));
            assert!(file.bytes > 0 && file.bytes <= 32 * 1024 * 1024);
            assert_eq!(file.sha256.len(), 64);
            assert!(file.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
        }
    }
    for vendor in ["KiCad", "Altium", "Cadence"] {
        assert!(
            vendors.get(vendor).is_some_and(|&count| count >= 2),
            "{vendor}"
        );
    }
}

fn filename(name: &str) -> &str {
    Path::new(name).file_name().unwrap().to_str().unwrap()
}

fn kinds(document: &Document) -> BTreeMap<LayerKind, usize> {
    let mut result = BTreeMap::new();
    for layer in &document.layers {
        *result.entry(layer.kind).or_insert(0) += 1;
    }
    result
}

fn warnings(document: &Document) -> Vec<String> {
    let mut warnings: Vec<_> = document
        .layers
        .iter()
        .flat_map(|layer| {
            layer
                .warnings
                .iter()
                .map(|warning| format!("{}: {warning}", filename(&layer.name)))
        })
        .collect();
    warnings.sort();
    warnings
}

fn close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.02, "{actual} != {expected}");
}

fn check(document: &Document, sample: &Sample) {
    let e = &sample.expected;
    assert!(
        document.errors.is_empty(),
        "{}: {:?}",
        sample.id,
        document.errors
    );
    assert_eq!(document.layers.len(), e.layers, "{}", sample.id);
    assert_eq!(document.primitives(), e.primitives, "{}", sample.id);
    assert_eq!(kinds(document), e.kinds, "{}", sample.id);
    assert_eq!(warnings(document), e.warnings, "{}", sample.id);
    for (actual, expected) in document.bounds.size().into_iter().zip(e.bounds_mm) {
        close(actual, expected);
    }
    for layer in &document.layers {
        assert_eq!(layer.visible, layer.kind.default_visible());
        let g = &layer.geometry;
        assert_eq!(g.indices.len() % 3, 0);
        assert!(
            g.vertices
                .iter()
                .all(|v| v.position.iter().all(|x| x.is_finite()))
        );
        assert!(
            g.indices
                .iter()
                .all(|&index| (index as usize) < g.vertices.len())
        );
        assert!(g.features.iter().all(|f| f.bounds.valid()));
        assert!(g.batches.iter().all(|b| b.bounds.valid()
            && b.indices.start <= b.indices.end
            && b.indices.end as usize <= g.indices.len()));
        if !g.bounds.valid() {
            assert!(g.indices.is_empty() && g.features.is_empty());
        }
    }
    if let Some(probe) = &e.drill {
        let layer = document
            .layers
            .iter()
            .find(|l| l.name.ends_with(&probe.suffix))
            .unwrap();
        assert_eq!(layer.kind, LayerKind::Drill);
        assert_eq!(layer.apertures, probe.tools);
        assert_eq!(layer.geometry.features.len(), probe.hits);
        let first = &layer.geometry.features[0];
        for (actual, expected) in first.bounds.center().into_iter().zip(probe.first_center) {
            close(actual, expected);
        }
        close(first.bounds.size()[0], probe.first_diameter);
        assert!(layer.geometry.pick(probe.first_center, 0.0).is_some());
        for (index, hits) in probe.tool_hits.iter().enumerate() {
            let prefix = format!("Drill · T{} ·", index + 1);
            assert_eq!(
                layer
                    .geometry
                    .features
                    .iter()
                    .filter(|f| f.label.starts_with(&prefix))
                    .count(),
                *hits
            );
        }
    }
}

fn compare(a: &Document, b: &Document) {
    assert_eq!(a.bounds, b.bounds);
    assert_eq!(a.triangles(), b.triangles());
    assert_eq!(a.layers.len(), b.layers.len());
    for (a, b) in a.layers.iter().zip(&b.layers) {
        assert_eq!(filename(&a.name), filename(&b.name));
        assert_eq!(a.kind, b.kind);
        assert_eq!(a.geometry.snap_shapes.len(), b.geometry.snap_shapes.len());
        assert_eq!(a.geometry.indices, b.geometry.indices);
        assert_eq!(a.geometry.vertices.len(), b.geometry.vertices.len());
        assert!(
            a.geometry
                .vertices
                .iter()
                .zip(&b.geometry.vertices)
                .all(|(a, b)| a.position == b.position)
        );
        assert_eq!(a.geometry.features.len(), b.geometry.features.len());
        assert!(
            a.geometry
                .features
                .iter()
                .zip(&b.geometry.features)
                .all(|(a, b)| a.bounds == b.bounds
                    && a.label == b.label
                    && a.dark == b.dark
                    && a.snap.shape == b.snap.shape
                    && a.snap.offset == b.snap.offset
                    && a.snap.center == b.snap.center)
        );
        for feature in a.geometry.features.iter().take(8) {
            let p = feature.snap.center.unwrap_or(feature.bounds.center());
            for mode in [
                gerbrr::snapping::SnapMode::Auto,
                gerbrr::snapping::SnapMode::Edges,
            ] {
                assert_eq!(a.geometry.snap(p, 0.1, mode), b.geometry.snap(p, 0.1, mode));
            }
        }
    }
}

fn read_verified(sample: &Sample) -> (PathBuf, Vec<(String, Vec<u8>)>) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/corpus")
        .join(&sample.id);
    let mut inputs = Vec::new();
    for file in &sample.files {
        let path = root
            .join(if file.role == "input" {
                "inputs"
            } else {
                "provenance"
            })
            .join(filename(&file.source));
        let bytes = std::fs::read(&path).unwrap_or_else(|e| {
            panic!("{}: {e}. Run mise run corpus-fetch first.", path.display())
        });
        assert_eq!(bytes.len(), file.bytes, "{}", path.display());
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            file.sha256,
            "{}",
            path.display()
        );
        if file.role == "input" {
            inputs.push((filename(&file.source).to_owned(), bytes));
        }
    }
    (root.join("inputs"), inputs)
}

#[test]
#[ignore = "requires mise run corpus-fetch; upstream artwork is not bundled"]
fn native_browser_and_worker_imports_match_all_pinned_public_samples() {
    // Sequential: large panelized samples can hold hundreds of MiB during transfer.
    for sample in manifest().samples {
        let (directory, inputs) = read_verified(&sample);
        let native =
            document::load(vec![directory]).unwrap_or_else(|e| panic!("{}: {e}", sample.id));
        check(&native, &sample);
        let browser = document::load_bytes(inputs.clone()).unwrap();
        check(&browser, &sample);
        compare(&native, &browser);
        let decoded = transfer::decode(&transfer::encode(&browser).unwrap()).unwrap();
        check(&decoded, &sample);
        compare(&native, &decoded);
        if !inputs.iter().any(|(name, _)| name.ends_with(".zip")) {
            // Exercise Cadence folder exports through the browser ZIP path too,
            // including the precision sidecar. No upstream archive is extracted.
            let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
            for (name, bytes) in inputs {
                zip.start_file(
                    format!("MFG/{name}"),
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
                zip.write_all(&bytes).unwrap();
            }
            let bytes = zip.finish().unwrap().into_inner();
            let zipped = document::load_bytes(vec![(format!("{}.zip", sample.id), bytes)]).unwrap();
            check(&zipped, &sample);
            compare(&native, &zipped);
        }
        eprintln!(
            "{}: {} layers · {} primitives · {} expected warnings",
            sample.id,
            native.layers.len(),
            native.primitives(),
            native.warning_count()
        );
    }
}
