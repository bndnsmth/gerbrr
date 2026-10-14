use gerbrr::snapping::SnapMode;
use gerbrr::{document, parser};
use std::hint::black_box;
use std::time::Instant;

fn main() {
    println!("Gerbrr parser + tessellation + spatial-index benchmark (CPU only)");
    let mut source = String::from("%FSLAX36Y36*%%MOMM*%%ADD10C,0.3*%D10*");
    for i in 0..100_000 {
        let x = (i % 1000) * 500_000;
        let y = (i / 1000) * 500_000;
        source.push_str(&format!("X{x}Y{y}D03*"));
    }
    source.push_str("M02*");
    black_box(parser::parse_gerber(&source).unwrap()); // warm-up
    let start = Instant::now();
    let mut triangles = 0;
    for _ in 0..5 {
        let parsed = black_box(parser::parse_gerber(black_box(&source)).unwrap());
        assert_eq!(parsed.geometry.features.len(), 100_000);
        triangles = parsed.geometry.indices.len() / 3;
    }
    let elapsed = start.elapsed().as_secs_f64() / 5.0;
    println!(
        "100,000 flashes: {:.2} ms · {:.2} M primitives/s · {} triangles",
        elapsed * 1000.0,
        0.1 / elapsed,
        triangles
    );
    let start = Instant::now();
    for _ in 0..100 {
        black_box(document::demo());
    }
    println!(
        "Six-layer demo: {:.2} ms average",
        start.elapsed().as_secs_f64() * 10.0
    );
    let parsed = parser::parse_gerber(&source).unwrap();
    let start = Instant::now();
    for i in 0..20_000 {
        let p = [
            (i % 1000) as f32 * 0.5 + 0.01,
            (i / 1000) as f32 * 0.5 + 0.01,
        ];
        assert!(black_box(parsed.geometry.snap(black_box(p), 0.05, SnapMode::Auto)).is_some());
    }
    println!(
        "100,000-flash center snap: {:.2} µs/query · {} shared source shapes",
        start.elapsed().as_secs_f64() * 1_000_000.0 / 20_000.0,
        parsed.geometry.snap_shapes.len()
    );
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/corpus/altium-limesdr-mini/inputs");
    if path.exists() {
        let doc = document::load(vec![path]).unwrap();
        let selected = doc
            .layers
            .iter()
            .position(|l| l.kind == document::LayerKind::Copper)
            .unwrap();
        let points: Vec<_> = doc.layers[selected]
            .geometry
            .features
            .iter()
            .filter_map(|f| f.snap.center)
            .take(500)
            .collect();
        for mode in [SnapMode::Auto, SnapMode::Edges] {
            let start = Instant::now();
            for _ in 0..20 {
                for p in &points {
                    black_box(doc.snap([p[0] + 0.003, p[1] + 0.003], 0.06, mode, selected));
                }
            }
            println!(
                "LimeSDR Mini {} snap: {:.2} µs/query (visible layers)",
                mode.label(),
                start.elapsed().as_secs_f64() * 1_000_000.0 / (points.len() * 20) as f64
            );
        }
    }
    println!("These are synthetic import timings, not FPS or comparative claims.");
}
