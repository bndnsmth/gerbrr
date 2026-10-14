//! Measurement targets come from source geometry, never mesh diagonals or feature boxes.
use crate::geometry::{Bounds, Point};
use lyon::path::{Event, Path, iterator::PathIterator};
use rstar::{AABB, RTree, RTreeObject};
use std::ops::Range;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SnapMode {
    #[default]
    Auto,
    Centers,
    Edges,
}
impl SnapMode {
    pub fn next(self) -> Self {
        match self {
            Self::Auto => Self::Centers,
            Self::Centers => Self::Edges,
            Self::Edges => Self::Auto,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Centers => "Centers",
            Self::Edges => "Edges",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SnapKind {
    Center,
    Endpoint,
    Edge,
}
impl SnapKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Center => "Center",
            Self::Endpoint => "Endpoint",
            Self::Edge => "Edge",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapHit {
    pub point: Point,
    pub kind: SnapKind,
    pub feature: usize,
    /// Outward direction used to display the tangent at an edge target.
    pub normal: Point,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SnapRef {
    pub shape: usize,
    pub offset: Point,
    /// Declared flash/drill origin, or an explicitly defined straight-slot center.
    pub center: Option<Point>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct SnapShape {
    segments: Vec<[Point; 2]>,
    stroke_radius: f32,
    circle: Option<(Point, f32, f32)>,
    #[serde(skip)]
    chunks: RTree<EdgeChunk>,
}

#[derive(Clone)]
struct EdgeChunk {
    range: Range<usize>,
    envelope: AABB<Point>,
}
impl RTreeObject for EdgeChunk {
    type Envelope = AABB<Point>;
    fn envelope(&self) -> Self::Envelope {
        self.envelope
    }
}

pub(crate) fn distance_squared(a: Point, b: Point) -> f32 {
    (a[0] - b[0]).mul_add(a[0] - b[0], (a[1] - b[1]) * (a[1] - b[1]))
}
pub(crate) fn add(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1]]
}
pub(crate) fn subtract(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}

fn project(p: Point, a: Point, b: Point) -> Point {
    let d = subtract(b, a);
    let length = d[0].mul_add(d[0], d[1] * d[1]);
    if length == 0.0 {
        return a;
    }
    let t = ((p[0] - a[0]).mul_add(d[0], (p[1] - a[1]) * d[1]) / length).clamp(0.0, 1.0);
    [a[0] + d[0] * t, a[1] + d[1] * t]
}
fn unit(v: Point) -> Point {
    let length = v[0].hypot(v[1]);
    if length > 0.0 {
        [v[0] / length, v[1] / length]
    } else {
        [1.0, 0.0]
    }
}

impl SnapShape {
    pub fn fill(path: &Path) -> Self {
        let mut segments = Vec::new();
        // Match the renderer's 5 µm curve tolerance, closing all fill subpaths.
        for event in path.iter().flattened(0.005) {
            let edge = match event {
                Event::Line { from, to } => Some([from.to_array(), to.to_array()]),
                Event::End { last, first, .. } => Some([last.to_array(), first.to_array()]),
                _ => None,
            };
            if let Some([a, b]) = edge
                && a != b
            {
                segments.push([a, b]);
            }
        }
        let mut result = Self {
            segments,
            stroke_radius: 0.0,
            circle: None,
            chunks: RTree::new(),
        };
        result.rebuild_index();
        result
    }

    pub fn circle(center: Point, radius: f32, hole: f32) -> Self {
        Self {
            segments: Vec::new(),
            stroke_radius: 0.0,
            circle: Some((center, radius, hole)),
            chunks: RTree::new(),
        }
    }

    pub fn stroke(points: &[Point], width: f32) -> Self {
        let mut result = Self {
            segments: points
                .windows(2)
                .filter(|p| p[0] != p[1])
                .map(|p| [p[0], p[1]])
                .collect(),
            stroke_radius: width * 0.5,
            circle: None,
            chunks: RTree::new(),
        };
        result.rebuild_index();
        result
    }

    pub(crate) fn rebuild_index(&mut self) {
        // Index groups of 32 edges, rather than millions of repeated pad edges.
        // Small templates are faster to scan; large planes get local/ray queries.
        self.chunks = if self.segments.len() <= 64 {
            RTree::new()
        } else {
            RTree::bulk_load(
                self.segments
                    .chunks(32)
                    .enumerate()
                    .map(|(index, edges)| {
                        let mut bounds = Bounds::default();
                        for edge in edges {
                            bounds.include(edge[0]);
                            bounds.include(edge[1]);
                        }
                        EdgeChunk {
                            range: index * 32..(index * 32 + edges.len()),
                            envelope: AABB::from_corners(bounds.min, bounds.max),
                        }
                    })
                    .collect(),
            )
        };
    }

    fn visit_segments(&self, bounds: Bounds, mut visit: impl FnMut([Point; 2])) {
        if self.chunks.size() == 0 {
            for &edge in &self.segments {
                visit(edge);
            }
        } else {
            for chunk in self
                .chunks
                .locate_in_envelope_intersecting(&AABB::from_corners(bounds.min, bounds.max))
            {
                for &edge in &self.segments[chunk.range.clone()] {
                    visit(edge);
                }
            }
        }
    }

    pub(crate) fn contains(&self, p: Point) -> bool {
        if let Some((center, radius, hole)) = self.circle {
            let d = distance_squared(p, center);
            return (d <= radius * radius) ^ (hole > 0.0 && d < hole * hole);
        }
        if self.stroke_radius > 0.0 {
            let r = self.stroke_radius;
            let mut inside = false;
            self.visit_segments(
                Bounds {
                    min: [p[0] - r, p[1] - r],
                    max: [p[0] + r, p[1] + r],
                },
                |[a, b]| {
                    inside |= distance_squared(p, project(p, a, b)) <= r * r;
                },
            );
            return inside;
        }
        let mut inside = false;
        self.visit_segments(
            Bounds {
                min: p,
                max: [1_000_001.0, p[1]],
            },
            |[a, b]| {
                if (a[1] > p[1]) != (b[1] > p[1]) {
                    let x = a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
                    if x > p[0] {
                        inside = !inside;
                    }
                }
            },
        );
        inside
    }

    pub(crate) fn visit_edges(&self, p: Point, radius: f32, mut visit: impl FnMut(Point, Point)) {
        let mut candidate = |point: Point, normal: Point| {
            if distance_squared(point, p) <= radius * radius {
                visit(point, normal);
            }
        };
        if let Some((center, r, hole)) = self.circle {
            let normal = unit(subtract(p, center));
            for radius in [r, hole].into_iter().filter(|&r| r > 0.0) {
                candidate(
                    add(center, [normal[0] * radius, normal[1] * radius]),
                    normal,
                );
            }
            return;
        }
        let r = radius + self.stroke_radius;
        self.visit_segments(
            Bounds {
                min: [p[0] - r, p[1] - r],
                max: [p[0] + r, p[1] + r],
            },
            |[a, b]| {
                let q = project(p, a, b);
                if self.stroke_radius > 0.0 {
                    let normal = if q == p {
                        unit([b[1] - a[1], a[0] - b[0]])
                    } else {
                        unit(subtract(p, q))
                    };
                    for sign in [-1.0, 1.0] {
                        candidate(
                            add(
                                q,
                                [
                                    normal[0] * self.stroke_radius * sign,
                                    normal[1] * self.stroke_radius * sign,
                                ],
                            ),
                            [normal[0] * sign, normal[1] * sign],
                        );
                    }
                } else {
                    candidate(q, unit([b[1] - a[1], a[0] - b[0]]));
                }
            },
        );
    }

    pub(crate) fn endpoints(&self) -> impl Iterator<Item = Point> + '_ {
        let start = (self.stroke_radius > 0.0)
            .then(|| self.segments.first().map(|s| s[0]))
            .flatten();
        let end = (self.stroke_radius > 0.0)
            .then(|| self.segments.last().map(|s| s[1]))
            .flatten();
        start
            .into_iter()
            .chain(end.filter(|end| Some(*end) != start))
    }

    pub(crate) fn visit_witnesses(&self, mut visit: impl FnMut(Point, Point)) {
        if let Some((center, r, hole)) = self.circle {
            for normal in [[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0]] {
                for radius in [r, hole].into_iter().filter(|&r| r > 0.0) {
                    visit(
                        add(center, [normal[0] * radius, normal[1] * radius]),
                        normal,
                    );
                }
            }
        } else {
            // A bounded visibility probe for origins inside aperture holes or
            // outside asymmetric macros. A failed probe never invents a target.
            for &[a, b] in self.segments.iter().take(16) {
                let middle = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
                let normal = unit([b[1] - a[1], a[0] - b[0]]);
                let edge = add(
                    middle,
                    [
                        normal[0] * self.stroke_radius,
                        normal[1] * self.stroke_radius,
                    ],
                );
                visit(edge, normal);
            }
        }
    }

    pub(crate) fn epsilon(&self) -> f32 {
        let width = if let Some((_, r, hole)) = self.circle {
            (r - hole).abs().min(r)
        } else if self.stroke_radius > 0.0 {
            self.stroke_radius
        } else {
            1.0
        };
        (width * 0.01).clamp(0.00001, 0.001)
    }

    pub(crate) fn valid(&self) -> bool {
        let valid_point = |p: Point| p.iter().all(|v| v.is_finite() && v.abs() <= 1_000_001.0);
        self.stroke_radius.is_finite()
            && self.stroke_radius >= 0.0
            && self.stroke_radius <= 1_000_000.0
            && self.segments.len() <= 8_100_000
            && self
                .segments
                .iter()
                .all(|[a, b]| valid_point(*a) && valid_point(*b))
            && self.circle.is_none_or(|(center, r, hole)| {
                valid_point(center)
                    && r.is_finite()
                    && r > 0.0
                    && r <= 1_000_000.0
                    && hole.is_finite()
                    && (0.0..=1_000_000.0).contains(&hole)
                    && self.segments.is_empty()
                    && self.stroke_radius == 0.0
            })
    }

    pub(crate) fn segment_count(&self) -> usize {
        self.segments.len()
    }
}
