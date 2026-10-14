//! Binary, transferable worker result. Source files never go to a server.
use crate::document::Document;
use std::sync::Arc;

const MAGIC: &[u8] = b"GERBRR02";
const LIMIT: usize = 512 * 1024 * 1024;

pub fn encode(doc: &Document) -> Result<Vec<u8>, String> {
    let payload =
        bincode::serde::encode_to_vec(doc, bincode::config::standard().with_fixed_int_encoding())
            .map_err(|e| format!("Could not encode worker result: {e}"))?;
    if payload.len() > LIMIT {
        return Err("Browser worker result exceeds 512 MiB".into());
    }
    let mut result = Vec::with_capacity(MAGIC.len() + payload.len());
    result.extend_from_slice(MAGIC);
    result.extend(payload);
    Ok(result)
}

pub fn decode(bytes: &[u8]) -> Result<Document, String> {
    let payload = bytes
        .strip_prefix(MAGIC)
        .ok_or("Invalid browser worker result")?;
    if payload.len() > LIMIT {
        return Err("Browser worker result exceeds 512 MiB".into());
    }
    let (mut doc, consumed): (Document, usize) = bincode::serde::decode_from_slice(
        payload,
        bincode::config::standard()
            .with_fixed_int_encoding()
            .with_limit::<LIMIT>(),
    )
    .map_err(|e| format!("Invalid browser worker result: {e}"))?;
    if consumed != payload.len()
        || doc.layers.is_empty()
        || doc.layers.len() > 256
        || !doc.bounds.valid()
    {
        return Err("Invalid browser worker document".into());
    }
    for layer in &mut doc.layers {
        let g = Arc::get_mut(&mut layer.geometry)
            .ok_or("Worker result unexpectedly contains shared geometry")?;
        if g.vertices.len() > 8_100_000
            || g.indices.len() > 33_000_000
            || g.features.len() > 2_000_000
            || g.snap_shapes.len() > g.features.len()
            || g.snap_shapes
                .iter()
                .map(|s| s.segment_count())
                .sum::<usize>()
                > 8_100_000
            || g.snap_shapes.iter().any(|s| !s.valid())
            || !g.indices.len().is_multiple_of(3)
            || g.indices.iter().any(|i| *i as usize >= g.vertices.len())
            || g.vertices
                .iter()
                .any(|v| v.position.iter().any(|p| !p.is_finite()))
            || g.features.iter().any(|f| {
                !f.bounds.valid()
                    || f.snap.shape >= g.snap_shapes.len()
                    || f.snap
                        .offset
                        .iter()
                        .any(|v| !v.is_finite() || v.abs() > 1_000_001.0)
                    || f.snap
                        .center
                        .is_some_and(|p| p.iter().any(|v| !v.is_finite() || v.abs() > 1_000_001.0))
            })
            || g.batches.iter().any(|b| {
                !b.bounds.valid()
                    || b.indices.start > b.indices.end
                    || b.indices.end as usize > g.indices.len()
            })
        {
            return Err("Invalid browser worker geometry".into());
        }
        g.intern_labels();
        g.rebuild_index();
    }
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn worker_round_trip_restores_geometry_and_inspection() {
        let doc = crate::document::demo();
        let decoded = decode(&encode(&doc).unwrap()).unwrap();
        assert_eq!(decoded.layers.len(), doc.layers.len());
        assert_eq!(decoded.triangles(), doc.triangles());
        assert_eq!(decoded.bounds, doc.bounds);
        let g = &decoded.layers[0].geometry;
        let p = g.features[0].bounds.center();
        assert!(g.pick(p, 0.01).is_some());
    }
    #[test]
    fn worker_result_rejects_corruption_and_trailing_data() {
        assert!(decode(b"invalid").is_err());
        let mut bytes = encode(&crate::document::demo()).unwrap();
        bytes.push(0);
        assert!(decode(&bytes).is_err());
    }
}
