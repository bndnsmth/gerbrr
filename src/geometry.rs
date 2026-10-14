use crate::snapping::{
    SnapHit, SnapKind, SnapMode, SnapRef, SnapShape, add, distance_squared, subtract,
};
use bytemuck::{Pod, Zeroable};
use lyon::math::{Box2D, point};
use lyon::path::{Path, Winding};
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, LineCap, LineJoin,
    StrokeOptions, StrokeTessellator, StrokeVertex, VertexBuffers,
};
use rstar::PointDistance;
use rstar::{AABB, RTree, RTreeObject};
use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

pub type Point = [f32; 2];

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Bounds {
    pub min: Point,
    pub max: Point,
}

impl Default for Bounds {
    fn default() -> Self {
        Self {
            min: [f32::INFINITY; 2],
            max: [f32::NEG_INFINITY; 2],
        }
    }
}

impl Bounds {
    pub fn include(&mut self, p: Point) {
        for (axis, value) in p.into_iter().enumerate() {
            self.min[axis] = self.min[axis].min(value);
            self.max[axis] = self.max[axis].max(value);
        }
    }

    pub fn union(&mut self, other: Self) {
        if other.valid() {
            self.include(other.min);
            self.include(other.max);
        }
    }

    pub fn valid(self) -> bool {
        self.min
            .iter()
            .chain(self.max.iter())
            .all(|v| v.is_finite())
            && self.max[0] >= self.min[0]
            && self.max[1] >= self.min[1]
    }

    pub fn center(self) -> Point {
        [
            (self.min[0] + self.max[0]) * 0.5,
            (self.min[1] + self.max[1]) * 0.5,
        ]
    }

    pub fn size(self) -> Point {
        [self.max[0] - self.min[0], self.max[1] - self.min[1]]
    }

    pub fn intersects(self, other: Self) -> bool {
        self.min[0] <= other.max[0]
            && self.max[0] >= other.min[0]
            && self.min[1] <= other.max[1]
            && self.max[1] >= other.min[1]
    }

    pub fn contains(self, p: Point, margin: f32) -> bool {
        p[0] >= self.min[0] - margin
            && p[0] <= self.max[0] + margin
            && p[1] >= self.min[1] - margin
            && p[1] <= self.max[1] + margin
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, serde::Serialize, serde::Deserialize)]
pub struct Vertex {
    pub position: Point,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Batch {
    pub indices: Range<u32>,
    pub dark: bool,
    pub bounds: Bounds,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Feature {
    pub bounds: Bounds,
    pub label: Arc<str>,
    pub dark: bool,
    pub snap: SnapRef,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct Geometry {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub batches: Vec<Batch>,
    pub bounds: Bounds,
    pub features: Vec<Feature>,
    pub snap_shapes: Vec<Arc<SnapShape>>,
    #[serde(skip)]
    spatial: RTree<SpatialFeature>,
    #[serde(skip)]
    centers: RTree<SpatialCenter>,
}

struct SpatialCenter {
    point: Point,
    index: usize,
}
impl RTreeObject for SpatialCenter {
    type Envelope = AABB<Point>;
    fn envelope(&self) -> Self::Envelope {
        AABB::from_point(self.point)
    }
}
impl PointDistance for SpatialCenter {
    fn distance_2(&self, point: &Point) -> f32 {
        distance_squared(self.point, *point)
    }
}

struct SpatialFeature {
    envelope: AABB<Point>,
    index: usize,
}
impl RTreeObject for SpatialFeature {
    type Envelope = AABB<Point>;
    fn envelope(&self) -> Self::Envelope {
        self.envelope
    }
}

impl Geometry {
    pub fn rebuild_index(&mut self) {
        for shape in &mut self.snap_shapes {
            if let Some(shape) = Arc::get_mut(shape) {
                shape.rebuild_index();
            }
        }
        self.spatial = RTree::bulk_load(
            self.features
                .iter()
                .enumerate()
                .map(|(index, f)| SpatialFeature {
                    envelope: AABB::from_corners(f.bounds.min, f.bounds.max),
                    index,
                })
                .collect(),
        );
        self.centers = RTree::bulk_load(
            self.features
                .iter()
                .enumerate()
                .filter_map(|(index, feature)| {
                    feature
                        .snap
                        .center
                        .map(|point| SpatialCenter { point, index })
                })
                .collect(),
        );
    }

    pub(crate) fn intern_labels(&mut self) {
        let mut labels = HashMap::<Arc<str>, Arc<str>>::new();
        for feature in &mut self.features {
            feature.label = labels
                .entry(feature.label.clone())
                .or_insert_with(|| feature.label.clone())
                .clone();
        }
    }
    /// Bounds-based inspection; deliberately not a copper-connectivity or exact-shape query.
    pub fn pick(&self, point: Point, margin: f32) -> Option<usize> {
        let envelope = AABB::from_corners(
            [point[0] - margin, point[1] - margin],
            [point[0] + margin, point[1] + margin],
        );
        self.spatial
            .locate_in_envelope_intersecting(&envelope)
            .min_by(|a, b| {
                let area = |f: &SpatialFeature| {
                    let size = self.features[f.index].bounds.size();
                    size[0] * size[1]
                };
                area(a).total_cmp(&area(b)).then(b.index.cmp(&a.index))
            })
            .map(|f| f.index)
    }

    /// Ordered source coverage, including transparent aperture holes and LP clears.
    pub fn covered(&self, point: Point) -> bool {
        let mut last = None;
        for candidate in self
            .spatial
            .locate_in_envelope_intersecting(&AABB::from_point(point))
        {
            if last.is_some_and(|index| candidate.index <= index) {
                continue;
            }
            let f = &self.features[candidate.index];
            if self.snap_shapes[f.snap.shape].contains(subtract(point, f.snap.offset)) {
                last = Some(candidate.index);
            }
        }
        last.is_some_and(|index| self.features[index].dark)
    }

    fn boundary(&self, point: Point, normal: Point, epsilon: f32) -> bool {
        let delta = [normal[0] * epsilon, normal[1] * epsilon];
        if self.covered(add(point, delta)) != self.covered(subtract(point, delta)) {
            return true;
        }
        // At a polygon corner an axis-aligned normal can lie on the adjacent
        // edge. Diagonal probes recognize the actual corner, not a mesh vertex.
        for normal in [[0.70710677, 0.70710677], [0.70710677, -0.70710677]] {
            let delta = [normal[0] * epsilon, normal[1] * epsilon];
            if self.covered(add(point, delta)) != self.covered(subtract(point, delta)) {
                return true;
            }
        }
        false
    }

    fn center_visible(&self, index: usize, point: Point) -> bool {
        let f = &self.features[index];
        let shape = &self.snap_shapes[f.snap.shape];
        if f.dark && shape.contains(subtract(point, f.snap.offset)) && self.covered(point) {
            return true;
        }
        let mut visible = false;
        let epsilon = shape.epsilon();
        shape.visit_witnesses(|local, normal| {
            if visible {
                return;
            }
            let world = add(local, f.snap.offset);
            if !f.dark {
                visible = self.boundary(world, normal, epsilon);
            } else {
                for sign in [-1.0, 1.0] {
                    let local = add(
                        local,
                        [normal[0] * epsilon * sign, normal[1] * epsilon * sign],
                    );
                    if shape.contains(local) && self.covered(add(local, f.snap.offset)) {
                        visible = true;
                        break;
                    }
                }
            }
        });
        visible
    }

    pub fn snap(&self, point: Point, radius: f32, mode: SnapMode) -> Option<SnapHit> {
        if !point.iter().all(|p| p.is_finite()) || !radius.is_finite() || radius <= 0.0 {
            return None;
        }
        // Origins are indexed independently: an offset macro's source origin
        // need not lie in its rendered bounding box.
        if mode != SnapMode::Edges {
            for (index, center) in self
                .centers
                .nearest_neighbor_iter(&point)
                .take_while(|c| distance_squared(c.point, point) <= radius * radius)
                .enumerate()
            {
                // Do not stall the UI probing thousands of erased origins at a
                // very distant zoom. No snap is safer than an arbitrary target.
                if index >= 256 {
                    return None;
                }
                if self.center_visible(center.index, center.point) {
                    return Some(SnapHit {
                        point: center.point,
                        kind: SnapKind::Center,
                        feature: center.index,
                        normal: [0.0; 2],
                    });
                }
            }
        }
        let envelope = AABB::from_corners(
            [point[0] - radius, point[1] - radius],
            [point[0] + radius, point[1] + radius],
        );
        if self
            .spatial
            .locate_in_envelope_intersecting(&envelope)
            .take(513)
            .count()
            > 512
        {
            return None;
        }
        let mut result: Option<SnapHit> = None;
        let mut consider = |hit: SnapHit| {
            let better = result.is_none_or(|old| {
                hit.kind
                    .cmp(&old.kind)
                    .then(
                        distance_squared(hit.point, point)
                            .total_cmp(&distance_squared(old.point, point)),
                    )
                    .then(old.feature.cmp(&hit.feature))
                    .then(hit.point[0].total_cmp(&old.point[0]))
                    .then(hit.point[1].total_cmp(&old.point[1]))
                    .is_lt()
            });
            if better {
                result = Some(hit);
            }
        };
        for candidate in self.spatial.locate_in_envelope_intersecting(&envelope) {
            let f = &self.features[candidate.index];
            let shape = &self.snap_shapes[f.snap.shape];
            if mode != SnapMode::Edges {
                for local in shape.endpoints() {
                    let p = add(local, f.snap.offset);
                    if distance_squared(p, point) <= radius * radius && self.covered(p) {
                        consider(SnapHit {
                            point: p,
                            kind: SnapKind::Endpoint,
                            feature: candidate.index,
                            normal: [0.0; 2],
                        });
                    }
                }
            }
            if mode != SnapMode::Centers {
                shape.visit_edges(subtract(point, f.snap.offset), radius, |p, normal| {
                    let p = add(p, f.snap.offset);
                    // Reject overlap seams, cleared-away features, and internal
                    // stroke joins. We snap to the layer's final painted boundary.
                    if self.boundary(p, normal, shape.epsilon()) {
                        consider(SnapHit {
                            point: p,
                            kind: SnapKind::Edge,
                            feature: candidate.index,
                            normal,
                        });
                    }
                });
            }
        }
        result
    }
}

pub struct MeshBuilder {
    buffers: VertexBuffers<Vertex, u32>,
    fill: FillTessellator,
    stroke: StrokeTessellator,
    geometry: Geometry,
    labels: HashMap<String, Arc<str>>,
    shape_ids: HashMap<usize, usize>,
}

struct SnapSource {
    shape: Arc<SnapShape>,
    offset: Point,
    center: Option<Point>,
}

impl Default for MeshBuilder {
    fn default() -> Self {
        Self {
            buffers: VertexBuffers::new(),
            fill: FillTessellator::new(),
            stroke: StrokeTessellator::new(),
            geometry: Geometry::default(),
            labels: HashMap::new(),
            shape_ids: HashMap::new(),
        }
    }
}

impl MeshBuilder {
    fn record(
        &mut self,
        vertex_start: usize,
        index_start: usize,
        dark: bool,
        label: &str,
        shape: Arc<SnapShape>,
        center: Option<Point>,
    ) {
        if self.buffers.indices.len() == index_start {
            return;
        }
        let mut bounds = Bounds::default();
        for vertex in &self.buffers.vertices[vertex_start..] {
            bounds.include(vertex.position);
        }
        self.record_bounds(
            bounds,
            index_start,
            dark,
            label,
            SnapSource {
                shape,
                offset: [0.0; 2],
                center,
            },
        );
    }

    fn record_bounds(
        &mut self,
        bounds: Bounds,
        index_start: usize,
        dark: bool,
        label: &str,
        source: SnapSource,
    ) {
        let SnapSource {
            shape,
            offset,
            center,
        } = source;
        self.geometry.bounds.union(bounds);
        let label = if let Some(label) = self.labels.get(label) {
            label.clone()
        } else {
            let shared: Arc<str> = label.into();
            self.labels.insert(label.to_owned(), shared.clone());
            shared
        };
        let key = Arc::as_ptr(&shape) as usize;
        let shape = *self.shape_ids.entry(key).or_insert_with(|| {
            let index = self.geometry.snap_shapes.len();
            self.geometry.snap_shapes.push(shape);
            index
        });
        self.geometry.features.push(Feature {
            bounds,
            label,
            dark,
            snap: SnapRef {
                shape,
                offset,
                center,
            },
        });
        let end = self.buffers.indices.len() as u32;
        if let Some(last) = self.geometry.batches.last_mut()
            && last.dark == dark
            && last.indices.end - last.indices.start < 65_536
        {
            last.indices.end = end;
            last.bounds.union(bounds);
            return;
        }
        self.geometry.batches.push(Batch {
            indices: index_start as u32..end,
            dark,
            bounds,
        });
    }

    /// Expand an already-tessellated aperture without running lyon again.
    pub fn append(
        &mut self,
        template: &Geometry,
        offset: Point,
        dark: bool,
        label: &str,
    ) -> Result<(), String> {
        self.check_limits()?;
        if template.indices.is_empty() {
            return Ok(());
        }
        let bounds = Bounds {
            min: [
                template.bounds.min[0] + offset[0],
                template.bounds.min[1] + offset[1],
            ],
            max: [
                template.bounds.max[0] + offset[0],
                template.bounds.max[1] + offset[1],
            ],
        };
        if !bounds.valid()
            || bounds
                .min
                .iter()
                .chain(bounds.max.iter())
                .any(|v| v.abs() > 1_000_000.0)
        {
            return Err("Aperture geometry exceeds the supported ±1 km range".into());
        }
        let base = self.buffers.vertices.len() as u32;
        let index_start = self.buffers.indices.len();
        self.buffers
            .vertices
            .extend(template.vertices.iter().map(|v| Vertex {
                position: [v.position[0] + offset[0], v.position[1] + offset[1]],
            }));
        self.buffers
            .indices
            .extend(template.indices.iter().map(|i| *i + base));
        let source = template
            .features
            .first()
            .ok_or("Aperture template has no source shape")?;
        self.record_bounds(
            bounds,
            index_start,
            dark,
            label,
            SnapSource {
                shape: template.snap_shapes[source.snap.shape].clone(),
                offset: add(offset, source.snap.offset),
                center: Some(offset),
            },
        );
        Ok(())
    }

    pub fn fill(&mut self, path: &Path, dark: bool, label: &str) -> Result<(), String> {
        self.fill_shape(path, dark, label, Arc::new(SnapShape::fill(path)), None)
    }

    pub fn fill_circle(
        &mut self,
        center: Point,
        diameter: f32,
        hole: f32,
        dark: bool,
        label: &str,
    ) -> Result<(), String> {
        self.fill_shape(
            &circle(center, diameter, hole),
            dark,
            label,
            Arc::new(SnapShape::circle(center, diameter * 0.5, hole * 0.5)),
            Some(center),
        )
    }

    fn fill_shape(
        &mut self,
        path: &Path,
        dark: bool,
        label: &str,
        shape: Arc<SnapShape>,
        center: Option<Point>,
    ) -> Result<(), String> {
        self.check_limits()?;
        if !shape.valid() {
            return Err("Invalid snapping source geometry".into());
        }
        let v = self.buffers.vertices.len();
        let i = self.buffers.indices.len();
        let result = self.fill.tessellate_path(
            path,
            &FillOptions::default()
                .with_tolerance(0.005)
                .with_fill_rule(FillRule::EvenOdd),
            &mut BuffersBuilder::new(&mut self.buffers, |v: FillVertex<'_>| Vertex {
                position: v.position().to_array(),
            }),
        );
        if let Err(error) = result {
            self.buffers.vertices.truncate(v);
            self.buffers.indices.truncate(i);
            return Err(format!("Could not tessellate {label}: {error:?}"));
        }
        self.record(v, i, dark, label, shape, center);
        Ok(())
    }

    pub fn stroke(
        &mut self,
        points: &[Point],
        width: f32,
        dark: bool,
        label: &str,
    ) -> Result<(), String> {
        self.check_limits()?;
        if points.len() < 2 || width <= 0.0 {
            return Ok(());
        }
        let mut path = Path::builder();
        path.begin(point(points[0][0], points[0][1]));
        for p in &points[1..] {
            path.line_to(point(p[0], p[1]));
        }
        path.end(false);
        let v = self.buffers.vertices.len();
        let i = self.buffers.indices.len();
        let result = self.stroke.tessellate_path(
            &path.build(),
            &StrokeOptions::default()
                .with_line_width(width)
                .with_tolerance(0.005)
                .with_line_cap(LineCap::Round)
                .with_line_join(LineJoin::Round),
            &mut BuffersBuilder::new(&mut self.buffers, |v: StrokeVertex<'_, '_>| Vertex {
                position: v.position().to_array(),
            }),
        );
        if let Err(error) = result {
            self.buffers.vertices.truncate(v);
            self.buffers.indices.truncate(i);
            return Err(format!("Could not tessellate {label}: {error:?}"));
        }
        self.record(
            v,
            i,
            dark,
            label,
            Arc::new(SnapShape::stroke(points, width)),
            None,
        );
        Ok(())
    }

    pub fn stroke_slot(
        &mut self,
        a: Point,
        b: Point,
        width: f32,
        label: &str,
    ) -> Result<(), String> {
        let before = self.geometry.features.len();
        self.stroke(&[a, b], width, true, label)?;
        if self.geometry.features.len() > before {
            self.geometry.features.last_mut().unwrap().snap.center =
                Some([(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5]);
        }
        Ok(())
    }

    pub fn finish(mut self) -> Geometry {
        self.geometry.vertices = self.buffers.vertices;
        self.geometry.indices = self.buffers.indices;
        self.geometry.rebuild_index();
        self.geometry
    }

    fn check_limits(&self) -> Result<(), String> {
        if self.buffers.vertices.len() > 8_000_000
            || self.buffers.indices.len() > 32_000_000
            || self.geometry.features.len() >= 2_000_000
        {
            Err(
                "Layer exceeds the mesh safety limit (8M vertices / 32M indices / 2M primitives)"
                    .into(),
            )
        } else {
            Ok(())
        }
    }
}

pub fn circle(center: Point, diameter: f32, hole: f32) -> Path {
    let mut p = Path::builder();
    p.add_circle(
        point(center[0], center[1]),
        diameter / 2.0,
        Winding::Positive,
    );
    if hole > 0.0 {
        p.add_circle(point(center[0], center[1]), hole / 2.0, Winding::Negative);
    }
    p.build()
}

pub fn rectangle(center: Point, width: f32, height: f32, hole: f32) -> Path {
    let mut p = Path::builder();
    p.add_rectangle(
        &Box2D::new(
            point(center[0] - width / 2.0, center[1] - height / 2.0),
            point(center[0] + width / 2.0, center[1] + height / 2.0),
        ),
        Winding::Positive,
    );
    if hole > 0.0 {
        p.add_circle(point(center[0], center[1]), hole / 2.0, Winding::Negative);
    }
    p.build()
}

pub fn contours(contours: &[Vec<Point>]) -> Path {
    let mut p = Path::builder();
    for contour in contours {
        if let Some(first) = contour.first() {
            p.begin(point(first[0], first[1]));
            for v in &contour[1..] {
                p.line_to(point(v[0], v[1]));
            }
            p.end(true);
        }
    }
    p.build()
}

pub fn rotate(p: Point, angle: f32) -> Point {
    let (s, c) = angle.to_radians().sin_cos();
    [p[0] * c - p[1] * s, p[0] * s + p[1] * c]
}
