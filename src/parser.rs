//! RS-274X/X2 and Excellon parsing. Coordinates are normalized to millimeters.
//! Unsupported commands are reported, never silently claimed as supported.
use crate::geometry::{self, Geometry, MeshBuilder, Point};
use geo::{BooleanOps, Coord, LineString, MapCoords, MultiPolygon, Polygon};
use std::collections::HashMap;
use std::f32::consts::{PI, TAU};
use std::sync::Arc;

pub struct Parsed {
    pub geometry: Geometry,
    pub warnings: Vec<String>,
    pub file_function: Option<String>,
    pub apertures: usize,
}

/// Companion Gerber metadata can establish an otherwise-undeclared drill integer
/// width. Decimal precision is deliberately not assumed for trailing-zero data.
#[derive(Clone, Copy, Debug)]
pub struct GerberFormatHint {
    pub integer: usize,
    pub decimal: usize,
    pub unit_mm: f32,
}

/// Explicit Cadence NC export settings, not a guess from artwork precision.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrillFormatHint {
    pub integer: usize,
    pub decimal: usize,
    pub unit_mm: f32,
    pub trailing_zeros_suppressed: bool,
}

pub fn cadence_nc_format(source: &str) -> Option<DrillFormatHint> {
    let settings: HashMap<_, _> = source
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            Some((words.next()?, words.next()?))
        })
        .collect();
    let integer = settings.get("INTEGER-PLACES")?.parse().ok()?;
    let decimal = settings.get("DECIMAL-PLACES")?.parse().ok()?;
    if !(1..=9).contains(&integer) || !(1..=9).contains(&decimal) {
        return None;
    }
    let unit_mm = match *settings.get("OUTPUT-UNITS")? {
        "ENGLISH" | "INCHES" => 25.4,
        "METRIC" | "MILLIMETERS" => 1.0,
        _ => return None,
    };
    let leading = match *settings.get("SUPPRESS-LEAD-ZEROES")? {
        "YES" => true,
        "NO" => false,
        _ => return None,
    };
    let trailing = match *settings.get("SUPPRESS-TRAIL-ZEROES")? {
        "YES" => true,
        "NO" => false,
        _ => return None,
    };
    if leading && trailing {
        return None;
    }
    Some(DrillFormatHint {
        integer,
        decimal,
        unit_mm,
        trailing_zeros_suppressed: trailing,
    })
}

pub fn gerber_format_hint(source: &str) -> Option<GerberFormatHint> {
    let mut format = None;
    let mut unit = None;
    for block in source.split('%').skip(1).step_by(2).take(16) {
        for parameter in block.split('*').map(str::trim) {
            if parameter.starts_with("FS") {
                let words = fields(parameter);
                let x = words.iter().find(|(k, _)| *k == b'X')?.1;
                let y = words.iter().find(|(k, _)| *k == b'Y')?.1;
                if x != y || x.len() != 2 || !x.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                let bytes = x.as_bytes();
                format = Some(((bytes[0] - b'0') as usize, (bytes[1] - b'0') as usize));
            } else if parameter == "MOIN" {
                unit = Some(25.4);
            } else if parameter == "MOMM" {
                unit = Some(1.0);
            }
            if let (Some((integer, decimal)), Some(unit_mm)) = (format, unit) {
                return Some(GerberFormatHint {
                    integer,
                    decimal,
                    unit_mm,
                });
            }
        }
    }
    None
}

#[derive(Clone)]
enum Aperture {
    Circle(f32, f32),
    Shape(Vec<Vec<Point>>),
}

#[derive(Clone, Copy)]
struct Format {
    integer: usize,
    decimal: usize,
    trailing: bool,
}

impl Format {
    fn coordinate(self, raw: &str, unit: f32) -> Result<f32, String> {
        let value = if raw.contains('.') {
            raw.parse::<f32>()
                .map_err(|_| format!("Invalid coordinate {raw:?}"))?
        } else {
            let digits = raw.trim_start_matches(['+', '-']).len();
            let exponent = if self.trailing {
                // LZ Excellon output keeps its integer-side zeros and may emit
                // arbitrary decimal precision (Cadence does this per coordinate).
                // Padding to a fixed maximum silently scales longer fields by 10×.
                self.integer as i32 - digits as i32
            } else {
                -(self.decimal as i32)
            };
            raw.parse::<f32>()
                .map_err(|_| format!("Invalid coordinate {raw:?}"))?
                * 10_f32.powi(exponent)
        } * unit;
        if !value.is_finite() || value.abs() > 1_000_000.0 {
            return Err("Coordinate exceeds the supported ±1 km range".into());
        }
        Ok(value)
    }
}

/// Returns all letter/number fields, including repeated G words.
fn fields(command: &str) -> smallvec::SmallVec<[(u8, &str); 8]> {
    let bytes = command.as_bytes();
    let mut result = smallvec::SmallVec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_alphabetic() {
            let letter = bytes[i];
            let start = i + 1;
            i = start;
            while i < bytes.len()
                && (bytes[i].is_ascii_digit() || matches!(bytes[i], b'.' | b'+' | b'-'))
            {
                i += 1;
            }
            if start < i {
                result.push((letter, &command[start..i]));
            }
        } else {
            i += 1;
        }
    }
    result
}

fn polygon(center: Point, diameter: f32, count: usize, rotation: f32) -> Vec<Point> {
    (0..count)
        .map(|i| {
            let a = rotation.to_radians() + TAU * i as f32 / count as f32;
            [
                center[0] + diameter * 0.5 * a.cos(),
                center[1] + diameter * 0.5 * a.sin(),
            ]
        })
        .collect()
}

fn box_points(center: Point, w: f32, h: f32) -> Vec<Point> {
    vec![
        [center[0] - w / 2.0, center[1] - h / 2.0],
        [center[0] + w / 2.0, center[1] - h / 2.0],
        [center[0] + w / 2.0, center[1] + h / 2.0],
        [center[0] - w / 2.0, center[1] + h / 2.0],
    ]
}

fn geo_polygon(points: &[Point]) -> Polygon<f64> {
    Polygon::new(
        LineString::new(
            points
                .iter()
                .map(|p| Coord {
                    x: p[0] as f64,
                    y: p[1] as f64,
                })
                .collect(),
        ),
        vec![],
    )
}

fn geo_contours(shape: MultiPolygon<f64>) -> Vec<Vec<Point>> {
    let mut result = Vec::new();
    for polygon in shape {
        for ring in std::iter::once(polygon.exterior()).chain(polygon.interiors()) {
            result.push(ring.0.iter().map(|p| [p.x as f32, p.y as f32]).collect());
        }
    }
    result
}

fn arc_points(
    start: Point,
    end: Point,
    offset: Point,
    clockwise: bool,
    single: bool,
) -> Vec<Point> {
    let mut center = [start[0] + offset[0], start[1] + offset[1]];
    if single {
        let mut best = f32::INFINITY;
        for sx in [-1.0, 1.0] {
            for sy in [-1.0, 1.0] {
                let c = [
                    start[0] + offset[0].abs() * sx,
                    start[1] + offset[1].abs() * sy,
                ];
                let r1 = (start[0] - c[0]).hypot(start[1] - c[1]);
                let r2 = (end[0] - c[0]).hypot(end[1] - c[1]);
                let a1 = (start[1] - c[1]).atan2(start[0] - c[0]);
                let a2 = (end[1] - c[1]).atan2(end[0] - c[0]);
                let sweep = if clockwise {
                    (a1 - a2).rem_euclid(TAU)
                } else {
                    (a2 - a1).rem_euclid(TAU)
                };
                let error = (r1 - r2).abs() + if sweep > PI / 2.0 + 0.001 { 1e6 } else { 0.0 };
                if error < best {
                    best = error;
                    center = c;
                }
            }
        }
    }
    let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
    let a1 = (start[1] - center[1]).atan2(start[0] - center[0]);
    let a2 = (end[1] - center[1]).atan2(end[0] - center[0]);
    let mut sweep = if clockwise {
        -((a1 - a2).rem_euclid(TAU))
    } else {
        (a2 - a1).rem_euclid(TAU)
    };
    if (start[0] - end[0]).hypot(start[1] - end[1]) < 1e-6 && !single {
        sweep = if clockwise { -TAU } else { TAU };
    }
    let step = (2.0 * (1.0 - 0.003 / radius.max(0.003)).clamp(-1.0, 1.0).acos()).max(0.001);
    let count = (sweep.abs() / step).ceil().clamp(1.0, 16384.0) as usize;
    let mut result: Vec<_> = (0..=count)
        .map(|i| {
            let a = a1 + sweep * i as f32 / count as f32;
            [center[0] + radius * a.cos(), center[1] + radius * a.sin()]
        })
        .collect();
    result[0] = start;
    result[count] = end;
    result
}

struct Gerber {
    mesh: MeshBuilder,
    apertures: HashMap<u32, Arc<Aperture>>,
    flashes: HashMap<(u32, u32, u32, bool, bool), Geometry>,
    macros: HashMap<String, Vec<String>>,
    warnings: Vec<String>,
    function: Option<String>,
    format: [Format; 2],
    unit: f32,
    position: Point,
    image_offset: Point,
    aperture: u32,
    operation: u32,
    interpolation: u32,
    single_quadrant: bool,
    incremental: bool,
    dark: bool,
    region: Option<Vec<Vec<Point>>>,
    repeat: (usize, usize, f32, f32),
    rotation: f32,
    scale: f32,
    mirror: Point,
    count: usize,
    block: bool,
}

impl Gerber {
    fn new() -> Self {
        Self {
            mesh: MeshBuilder::default(),
            apertures: HashMap::new(),
            flashes: HashMap::new(),
            macros: HashMap::new(),
            warnings: vec![],
            function: None,
            format: [Format {
                integer: 2,
                decimal: 4,
                trailing: false,
            }; 2],
            unit: 25.4,
            position: [0.0; 2],
            image_offset: [0.0; 2],
            aperture: 0,
            operation: 2,
            interpolation: 1,
            single_quadrant: false,
            incremental: false,
            dark: true,
            region: None,
            repeat: (1, 1, 0.0, 0.0),
            rotation: 0.0,
            scale: 1.0,
            mirror: [1.0; 2],
            count: 0,
            block: false,
        }
    }

    fn warn(&mut self, warning: impl Into<String>) {
        let w = warning.into();
        if self.warnings.len() < 100 && !self.warnings.contains(&w) {
            self.warnings.push(w);
        }
    }

    fn parameter(&mut self, command: &str) -> Result<(), String> {
        if command.starts_with("FS") {
            self.incremental = command.as_bytes().get(3) == Some(&b'I');
            for (axis, name) in ['X', 'Y'].into_iter().enumerate() {
                if let Some(i) = command.find(name) {
                    let bytes = command.as_bytes();
                    let integer = bytes.get(i + 1).and_then(|b| (*b as char).to_digit(10));
                    let decimal = bytes.get(i + 2).and_then(|b| (*b as char).to_digit(10));
                    if let (Some(integer), Some(decimal)) = (integer, decimal) {
                        self.format[axis] = Format {
                            integer: integer as usize,
                            decimal: decimal as usize,
                            trailing: command.as_bytes().get(2) == Some(&b'T'),
                        };
                    } else {
                        return Err("Invalid FS coordinate format".into());
                    }
                }
            }
        } else if command.starts_with("OF") {
            for (axis, value) in fields(command) {
                if matches!(axis, b'A' | b'B') {
                    let axis = usize::from(axis == b'B');
                    let offset = parse_number(value)? * self.unit;
                    self.position[axis] += offset - self.image_offset[axis];
                    self.image_offset[axis] = offset;
                }
            }
        } else if let Some(value) = command.strip_prefix("IR") {
            if parse_number(value)?.rem_euclid(360.0) != 0.0 {
                self.warn(format!(
                    "Unsupported non-identity image rotation: {command}"
                ));
            }
        } else if command.starts_with("MI") || command.starts_with("SF") {
            let identity = if command.starts_with("MI") { 0.0 } else { 1.0 };
            let words = fields(command);
            if !words.iter().any(|(k, _)| matches!(k, b'A' | b'B')) {
                return Err(format!("Invalid image transform: {command}"));
            }
            for (axis, value) in words {
                if matches!(axis, b'A' | b'B') && parse_number(value)? != identity {
                    self.warn(format!(
                        "Unsupported non-identity image transform: {command}"
                    ));
                    break;
                }
            }
        } else if command == "MOMM" {
            self.unit = 1.0;
        } else if command == "MOIN" {
            self.unit = 25.4;
        } else if let Some(definition) = command.strip_prefix("ADD") {
            let end = definition
                .find(|c: char| !c.is_ascii_digit())
                .ok_or("Missing aperture shape")?;
            let id: u32 = definition[..end]
                .parse()
                .map_err(|_| "Invalid aperture ID")?;
            let (shape, values) = definition[end..]
                .split_once(',')
                .unwrap_or((&definition[end..], ""));
            let args: Vec<f32> = values
                .split('X')
                .filter(|s| !s.is_empty())
                .map(|s| {
                    s.parse::<f32>()
                        .map_err(|_| format!("Invalid aperture parameter {s}"))
                })
                .collect::<Result<_, _>>()?;
            if args.iter().any(|a| !a.is_finite()) {
                return Err("Non-finite aperture parameter".into());
            }
            let get = |i: usize| args.get(i).copied().unwrap_or(0.0) * self.unit;
            let aperture = match shape {
                "C" => Aperture::Circle(get(0), get(1)),
                "R" => {
                    let mut c = vec![box_points([0.0; 2], get(0), get(1))];
                    if get(2) > 0.0 {
                        c.push(polygon([0.0; 2], get(2), 96, 0.0));
                    }
                    Aperture::Shape(c)
                }
                "O" => {
                    let w = get(0);
                    let h = get(1);
                    let r = w.min(h) / 2.0;
                    let mut points = Vec::new();
                    for i in 0..96 {
                        let a = TAU * i as f32 / 96.0;
                        points.push([
                            r * a.cos()
                                + if w > h {
                                    (w / 2.0 - r) * a.cos().signum()
                                } else {
                                    0.0
                                },
                            r * a.sin()
                                + if h > w {
                                    (h / 2.0 - r) * a.sin().signum()
                                } else {
                                    0.0
                                },
                        ]);
                    }
                    let mut c = vec![points];
                    if get(2) > 0.0 {
                        c.push(polygon([0.0; 2], get(2), 96, 0.0));
                    }
                    Aperture::Shape(c)
                }
                "P" => {
                    let n = args.get(1).copied().unwrap_or(3.0).clamp(3.0, 128.0) as usize;
                    let mut c = vec![polygon(
                        [0.0; 2],
                        get(0),
                        n,
                        args.get(2).copied().unwrap_or(0.0),
                    )];
                    if get(3) > 0.0 {
                        c.push(polygon([0.0; 2], get(3), 96, 0.0));
                    }
                    Aperture::Shape(c)
                }
                name => {
                    let lines = self
                        .macros
                        .get(name)
                        .ok_or_else(|| format!("Undefined aperture macro {name}"))?;
                    Aperture::Shape(evaluate_macro(lines, &args, self.unit)?)
                }
            };
            self.apertures.insert(id, Arc::new(aperture));
            self.flashes.clear();
        } else if command == "LPD" {
            self.dark = true;
        } else if command == "LPC" {
            self.dark = false;
        } else if let Some(value) = command.strip_prefix("LR") {
            self.rotation = parse_number(value)?;
        } else if let Some(value) = command.strip_prefix("LS") {
            self.scale = parse_number(value)?;
            if self.scale <= 0.0 {
                return Err("Aperture scale must be positive".into());
            }
        } else if let Some(value) = command.strip_prefix("LM") {
            self.mirror = [
                if value.contains('X') { -1.0 } else { 1.0 },
                if value.contains('Y') { -1.0 } else { 1.0 },
            ];
        } else if command.starts_with("SR") {
            self.repeat = (1, 1, 0.0, 0.0);
            for (k, v) in fields(command) {
                match k {
                    b'X' => self.repeat.0 = v.parse().map_err(|_| "Invalid step-repeat X count")?,
                    b'Y' => self.repeat.1 = v.parse().map_err(|_| "Invalid step-repeat Y count")?,
                    b'I' => self.repeat.2 = parse_number(v)? * self.unit,
                    b'J' => self.repeat.3 = parse_number(v)? * self.unit,
                    _ => {}
                }
            }
            if self.repeat.0 == 0
                || self.repeat.1 == 0
                || self.repeat.0.saturating_mul(self.repeat.1) > 10_000
            {
                return Err("Step-repeat count must be between 1 and 10,000".into());
            }
        } else if let Some(function) = command.strip_prefix("TF.FileFunction,") {
            self.function = Some(function.into());
        } else if command.starts_with("AB") {
            self.block = command != "AB";
            self.warn("Block apertures (AB) are not supported; block content was skipped");
        } else if command.starts_with(['T', 'I'])
            && (command.starts_with("TF")
                || command.starts_with("TA")
                || command.starts_with("TO")
                || command.starts_with("TD")
                || command.starts_with("IN"))
        {
            // X2 metadata and image name do not change geometry.
        } else if command == "IPPOS" || command == "ASAXBY" {
        } else if !command.is_empty() {
            self.warn(format!("Unsupported parameter: {command}"));
        }
        Ok(())
    }

    fn transform(&self, p: Point, offset: Point) -> Point {
        let p = geometry::rotate(
            [
                p[0] * self.scale * self.mirror[0],
                p[1] * self.scale * self.mirror[1],
            ],
            self.rotation,
        );
        [p[0] + offset[0], p[1] + offset[1]]
    }

    fn emit(&mut self, points: &[Point], flash: bool) -> Result<(), String> {
        let aperture = self
            .apertures
            .get(&self.aperture)
            .cloned()
            .ok_or_else(|| format!("Aperture D{} used before definition", self.aperture))?;
        let label = format!(
            "{} · D{}",
            if flash { "Pad" } else { "Trace" },
            self.aperture
        );
        let flash_key = (
            self.aperture,
            self.rotation.to_bits(),
            self.scale.to_bits(),
            self.mirror[0] < 0.0,
            self.mirror[1] < 0.0,
        );
        if flash && !self.flashes.contains_key(&flash_key) {
            let mut template = MeshBuilder::default();
            match aperture.as_ref() {
                Aperture::Circle(d, hole) => {
                    template.fill_circle([0.0; 2], d * self.scale, hole * self.scale, true, "")?;
                }
                Aperture::Shape(contours) => {
                    let c: Vec<_> = contours
                        .iter()
                        .map(|c| c.iter().map(|p| self.transform(*p, [0.0; 2])).collect())
                        .collect();
                    template.fill(&geometry::contours(&c), true, "")?;
                }
            }
            self.flashes.insert(flash_key, template.finish());
        }
        for y in 0..self.repeat.1 {
            for x in 0..self.repeat.0 {
                self.count += 1;
                if self.count > 2_000_000 {
                    return Err("Safety limit: more than two million primitives".into());
                }
                let offset = [x as f32 * self.repeat.2, y as f32 * self.repeat.3];
                if flash {
                    self.mesh.append(
                        &self.flashes[&flash_key],
                        [points[0][0] + offset[0], points[0][1] + offset[1]],
                        self.dark,
                        &label,
                    )?;
                    continue;
                }
                let points: Vec<Point> = points
                    .iter()
                    .map(|p| [p[0] + offset[0], p[1] + offset[1]])
                    .collect();
                match aperture.as_ref() {
                    Aperture::Circle(d, _) => {
                        self.mesh
                            .stroke(&points, d * self.scale, self.dark, &label)?
                    }
                    Aperture::Shape(contours) => {
                        if contours.len() != 1 {
                            self.warn("Drawing with compound/holed apertures is not supported; trace skipped");
                            continue;
                        }
                        // The sweep of a convex aperture along each segment is its convex hull.
                        let base = &contours[0];
                        if !is_convex(base) {
                            self.warn(
                                "Drawing with non-convex apertures is not supported; trace skipped",
                            );
                            continue;
                        }
                        for segment in points.windows(2) {
                            let vertices: Vec<_> = segment
                                .iter()
                                .flat_map(|p| base.iter().map(|v| self.transform(*v, *p)))
                                .collect();
                            self.mesh.fill(
                                &geometry::contours(&[convex_hull(vertices)]),
                                self.dark,
                                &label,
                            )?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn command(&mut self, command: &str) -> Result<bool, String> {
        if let Some(comment) = command
            .strip_prefix("G04")
            .or_else(|| command.strip_prefix("G4 "))
        {
            // X2-compatible attributes may be carried in ordinary comments.
            if let Some(attribute) = comment.trim().strip_prefix("#@!")
                && let Some(function) = attribute.trim_start().strip_prefix("TF.FileFunction,")
            {
                self.function = Some(function.into());
            }
            return Ok(false);
        }
        if command.is_empty() {
            return Ok(false);
        }
        if self.block {
            return Ok(false);
        }
        let words = fields(command);
        for &(k, v) in &words {
            if k == b'G' {
                let code = v.parse::<u32>().map_err(|_| "Invalid G-code")?;
                match code {
                    1..=3 => self.interpolation = code,
                    36 => self.region = Some(vec![]),
                    37 => {
                        if let Some(contours) = self.region.take() {
                            for y in 0..self.repeat.1 {
                                for x in 0..self.repeat.0 {
                                    let c: Vec<_> = contours
                                        .iter()
                                        .map(|c| {
                                            c.iter()
                                                .map(|p| {
                                                    [
                                                        p[0] + x as f32 * self.repeat.2,
                                                        p[1] + y as f32 * self.repeat.3,
                                                    ]
                                                })
                                                .collect()
                                        })
                                        .collect();
                                    self.mesh
                                        .fill(&geometry::contours(&c), self.dark, "Region")?;
                                }
                            }
                        } else {
                            self.warn("G37 without an open region");
                        }
                    }
                    74 => self.single_quadrant = true,
                    75 => self.single_quadrant = false,
                    70 => self.unit = 25.4,
                    71 => self.unit = 1.0,
                    90 => self.incremental = false,
                    91 => self.incremental = true,
                    54 | 55 => {}
                    _ => self.warn(format!("Unsupported G-code: G{code}")),
                }
            }
            if k == b'M' && matches!(v.parse::<u32>(), Ok(0..=2)) {
                return Ok(true);
            }
        }
        let mut target = self.position;
        let mut offset = [0.0; 2];
        let mut has_position = false;
        let mut explicit_operation = false;
        for (k, v) in words {
            match k {
                b'X' | b'Y' => {
                    let axis = usize::from(k == b'Y');
                    let value = self.format[axis].coordinate(v, self.unit)?;
                    target[axis] = value
                        + if self.incremental {
                            self.position[axis]
                        } else {
                            self.image_offset[axis]
                        };
                    has_position = true;
                }
                b'I' | b'J' => {
                    offset[usize::from(k == b'J')] =
                        self.format[usize::from(k == b'J')].coordinate(v, self.unit)?
                }
                b'D' => {
                    let code: u32 = v.parse().map_err(|_| "Invalid D-code")?;
                    if code >= 10 {
                        self.aperture = code;
                    } else if (1..=3).contains(&code) {
                        self.operation = code;
                        explicit_operation = true;
                    } else {
                        self.warn(format!("Unsupported D-code: D{code}"));
                    }
                }
                _ => {}
            }
        }
        if !has_position && !explicit_operation {
            return Ok(false);
        }
        if self.operation == 2 {
            if let Some(contours) = self.region.as_mut() {
                contours.push(vec![target]);
            }
        } else if self.operation == 3 {
            self.emit(&[target], true)?;
        } else if self.operation == 1 {
            let points = if self.interpolation == 1 {
                vec![self.position, target]
            } else {
                arc_points(
                    self.position,
                    target,
                    offset,
                    self.interpolation == 2,
                    self.single_quadrant,
                )
            };
            if let Some(contours) = self.region.as_mut() {
                if contours.is_empty() {
                    contours.push(vec![self.position]);
                }
                contours.last_mut().unwrap().extend_from_slice(&points[1..]);
            } else {
                self.emit(&points, false)?;
            }
        }
        self.position = target;
        Ok(false)
    }
}

pub fn parse_gerber(source: &str) -> Result<Parsed, String> {
    if source.len() > 128 * 1024 * 1024 {
        return Err("File exceeds the 128 MiB safety limit".into());
    }
    let source = source.trim_start_matches('\u{feff}');
    let mut parser = Gerber::new();
    let mut cursor = 0;
    let mut recognized = false;
    while cursor < source.len() {
        let rest = source[cursor..].trim_start();
        cursor = source.len() - rest.len();
        if rest.is_empty() {
            break;
        }
        if let Some(rest) = rest.strip_prefix('%') {
            let end = rest
                .find('%')
                .ok_or("Unterminated extended parameter block")?;
            let block = &rest[..end];
            let mut parts = block.split('*').map(str::trim).filter(|s| !s.is_empty());
            if let Some(first) = parts.next() {
                recognized |=
                    first.starts_with("FS") || first.starts_with("MO") || first.starts_with("AD");
                if let Some(name) = first.strip_prefix("AM") {
                    parser
                        .macros
                        .insert(name.into(), parts.map(str::to_owned).collect());
                } else {
                    parser.parameter(first)?;
                    for part in parts {
                        parser.parameter(part)?;
                    }
                }
            }
            cursor += end + 2;
        } else {
            let end = rest
                .find('*')
                .ok_or("Unterminated Gerber command (missing *)")?;
            let command = rest[..end].trim();
            if parser
                .command(command)
                .map_err(|e| format!("{e} near {command:.80}"))?
            {
                break;
            }
            cursor += end + 1;
        }
    }
    if !recognized {
        return Err("Not an extended Gerber (RS-274X) file".into());
    }
    if parser.region.is_some() {
        parser.warn("Unclosed G36 region; unfinished region was skipped");
    }
    Ok(Parsed {
        geometry: parser.mesh.finish(),
        warnings: parser.warnings,
        file_function: parser.function,
        apertures: parser.apertures.len(),
    })
}

fn parse_number(s: &str) -> Result<f32, String> {
    let v: f32 = s
        .trim()
        .parse()
        .map_err(|_| format!("Invalid number {s:?}"))?;
    if v.is_finite() {
        Ok(v)
    } else {
        Err("Non-finite number".into())
    }
}

fn cross(a: Point, b: Point, c: Point) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn convex_hull(mut p: Vec<Point>) -> Vec<Point> {
    p.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    p.dedup();
    let mut lower = Vec::new();
    for &v in &p {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], v) <= 0.0 {
            lower.pop();
        }
        lower.push(v);
    }
    let mut upper = Vec::new();
    for &v in p.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], v) <= 0.0 {
            upper.pop();
        }
        upper.push(v);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

fn is_convex(points: &[Point]) -> bool {
    if points.len() < 3 {
        return false;
    }
    let mut sign = 0.0;
    for i in 0..points.len() {
        let c = cross(
            points[i],
            points[(i + 1) % points.len()],
            points[(i + 2) % points.len()],
        );
        if c.abs() > 1e-6 {
            if sign != 0.0 && c.signum() != sign {
                return false;
            }
            sign = c.signum();
        }
    }
    true
}

/// Small expression evaluator for aperture macro arithmetic and $variables.
struct Expression<'a> {
    bytes: &'a [u8],
    cursor: usize,
    variables: &'a HashMap<u32, f32>,
}
impl Expression<'_> {
    fn whitespace(&mut self) {
        while self
            .bytes
            .get(self.cursor)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.cursor += 1;
        }
    }
    fn sum(&mut self) -> Result<f32, String> {
        let mut value = self.product()?;
        loop {
            self.whitespace();
            match self.bytes.get(self.cursor) {
                Some(b'+') => {
                    self.cursor += 1;
                    value += self.product()?;
                }
                Some(b'-') => {
                    self.cursor += 1;
                    value -= self.product()?;
                }
                _ => return Ok(value),
            }
        }
    }
    fn product(&mut self) -> Result<f32, String> {
        let mut value = self.atom()?;
        loop {
            self.whitespace();
            match self.bytes.get(self.cursor) {
                Some(b'x' | b'X') => {
                    self.cursor += 1;
                    value *= self.atom()?;
                }
                Some(b'/') => {
                    self.cursor += 1;
                    value /= self.atom()?;
                }
                _ => return Ok(value),
            }
        }
    }
    fn atom(&mut self) -> Result<f32, String> {
        self.whitespace();
        match self.bytes.get(self.cursor) {
            Some(b'-') => {
                self.cursor += 1;
                Ok(-self.atom()?)
            }
            Some(b'+') => {
                self.cursor += 1;
                self.atom()
            }
            Some(b'(') => {
                self.cursor += 1;
                let value = self.sum()?;
                self.whitespace();
                if self.bytes.get(self.cursor) != Some(&b')') {
                    return Err("Unclosed macro expression".into());
                }
                self.cursor += 1;
                Ok(value)
            }
            Some(b'$') => {
                self.cursor += 1;
                let start = self.cursor;
                while self.bytes.get(self.cursor).is_some_and(u8::is_ascii_digit) {
                    self.cursor += 1;
                }
                let id: u32 = std::str::from_utf8(&self.bytes[start..self.cursor])
                    .unwrap()
                    .parse()
                    .map_err(|_| "Invalid macro variable")?;
                Ok(*self.variables.get(&id).unwrap_or(&0.0))
            }
            _ => {
                let start = self.cursor;
                while self
                    .bytes
                    .get(self.cursor)
                    .is_some_and(|b| b.is_ascii_digit() || *b == b'.')
                {
                    self.cursor += 1;
                }
                parse_number(
                    std::str::from_utf8(&self.bytes[start..self.cursor])
                        .map_err(|_| "Invalid macro expression")?,
                )
            }
        }
    }
}

fn expression(s: &str, vars: &HashMap<u32, f32>) -> Result<f32, String> {
    let mut e = Expression {
        bytes: s.as_bytes(),
        cursor: 0,
        variables: vars,
    };
    let value = e.sum()?;
    e.whitespace();
    if e.cursor != e.bytes.len() || !value.is_finite() {
        return Err(format!("Invalid macro expression {s:?}"));
    }
    Ok(value)
}

fn evaluate_macro(lines: &[String], args: &[f32], unit: f32) -> Result<Vec<Vec<Point>>, String> {
    let mut vars: HashMap<_, _> = args
        .iter()
        .enumerate()
        .map(|(i, v)| (i as u32 + 1, *v))
        .collect();
    let mut shape = MultiPolygon::<f64>(vec![]);
    for line in lines {
        if line.starts_with('0') {
            continue;
        }
        if let Some((variable, value)) = line.split_once('=') {
            let id: u32 = variable
                .trim()
                .trim_start_matches('$')
                .parse()
                .map_err(|_| "Invalid macro assignment")?;
            vars.insert(id, expression(value, &vars)?);
            continue;
        }
        let values: Vec<f32> = line
            .split(',')
            .map(|v| expression(v, &vars))
            .collect::<Result<_, _>>()?;
        let get = |i: usize| -> Result<f32, String> {
            values
                .get(i)
                .copied()
                .ok_or_else(|| format!("Missing macro parameter in {line}"))
        };
        let code = get(0)? as u32;
        let mut add_shape = |poly: MultiPolygon<f64>, exposure: bool, rotation: f32| {
            let poly = poly.map_coords(|p| {
                let p = geometry::rotate([p.x as f32, p.y as f32], rotation);
                Coord {
                    x: (p[0] * unit) as f64,
                    y: (p[1] * unit) as f64,
                }
            });
            shape = if exposure {
                shape.union(&poly)
            } else {
                shape.difference(&poly)
            };
        };
        let mut add = |points: Vec<Point>, exposure: bool, rotation: f32| {
            add_shape(MultiPolygon(vec![geo_polygon(&points)]), exposure, rotation);
        };
        match code {
            1 => add(
                polygon([get(3)?, get(4)?], get(2)?, 128, 0.0),
                get(1)? != 0.0,
                values.get(5).copied().unwrap_or(0.0),
            ),
            20 => {
                let a = [get(3)?, get(4)?];
                let b = [get(5)?, get(6)?];
                let length = (b[0] - a[0]).hypot(b[1] - a[1]);
                if length > 0.0 {
                    let n = [
                        -(b[1] - a[1]) / length * get(2)? / 2.0,
                        (b[0] - a[0]) / length * get(2)? / 2.0,
                    ];
                    add(
                        vec![
                            [a[0] + n[0], a[1] + n[1]],
                            [b[0] + n[0], b[1] + n[1]],
                            [b[0] - n[0], b[1] - n[1]],
                            [a[0] - n[0], a[1] - n[1]],
                        ],
                        get(1)? != 0.0,
                        get(7)?,
                    );
                }
            }
            21 | 22 => {
                let mut center = [get(4)?, get(5)?];
                if code == 22 {
                    center[0] += get(2)? / 2.0;
                    center[1] += get(3)? / 2.0;
                }
                add(
                    box_points(center, get(2)?, get(3)?),
                    get(1)? != 0.0,
                    get(6)?,
                );
            }
            4 => {
                let n = get(2)?.clamp(1.0, 5000.0) as usize;
                let points: Vec<_> = (0..=n)
                    .map(|i| Ok([get(3 + i * 2)?, get(4 + i * 2)?]))
                    .collect::<Result<_, String>>()?;
                add(points, get(1)? != 0.0, get(5 + n * 2)?);
            }
            5 => add(
                polygon(
                    [get(3)?, get(4)?],
                    get(5)?,
                    get(2)?.clamp(3.0, 128.0) as usize,
                    0.0,
                ),
                get(1)? != 0.0,
                get(6)?,
            ),
            7 => {
                let c = [get(1)?, get(2)?];
                let d = get(3)?;
                let rot = get(6)?;
                // Build a thermal locally, so its gaps cannot clear earlier macro primitives.
                let outer = MultiPolygon(vec![geo_polygon(&polygon(c, d, 128, 0.0))]);
                let inner = MultiPolygon(vec![geo_polygon(&polygon(c, get(4)?, 128, 0.0))]);
                let cross = MultiPolygon(vec![geo_polygon(&box_points(c, d, get(5)?))])
                    .union(&MultiPolygon(vec![geo_polygon(&box_points(c, get(5)?, d))]));
                add_shape(outer.difference(&inner).difference(&cross), true, rot);
            }
            6 => {
                let c = [get(1)?, get(2)?];
                let mut d = get(3)?;
                let thick = get(4)?;
                let gap = get(5)?;
                for _ in 0..get(6)?.clamp(0.0, 100.0) as usize {
                    if d <= 0.0 {
                        break;
                    }
                    let ring = MultiPolygon(vec![geo_polygon(&polygon(c, d, 128, 0.0))])
                        .difference(&MultiPolygon(vec![geo_polygon(&polygon(
                            c,
                            (d - 2.0 * thick).max(0.0),
                            128,
                            0.0,
                        ))]));
                    add_shape(ring, true, get(9)?);
                    d -= 2.0 * (thick + gap);
                }
                add_shape(
                    MultiPolygon(vec![geo_polygon(&box_points(c, get(8)?, get(7)?))]),
                    true,
                    get(9)?,
                );
                add_shape(
                    MultiPolygon(vec![geo_polygon(&box_points(c, get(7)?, get(8)?))]),
                    true,
                    get(9)?,
                );
            }
            _ => return Err(format!("Unsupported aperture macro primitive {code}")),
        }
    }
    Ok(geo_contours(shape))
}

pub fn parse_excellon(source: &str) -> Result<Parsed, String> {
    parse_excellon_with_hint(source, None)
}

pub fn parse_excellon_with_hint(
    source: &str,
    hint: Option<GerberFormatHint>,
) -> Result<Parsed, String> {
    parse_excellon_with_hints(source, hint, None)
}

pub fn parse_excellon_with_hints(
    source: &str,
    hint: Option<GerberFormatHint>,
    nc_hint: Option<DrillFormatHint>,
) -> Result<Parsed, String> {
    if source.len() > 128 * 1024 * 1024 {
        return Err("File exceeds the 128 MiB safety limit".into());
    }
    let source = source.trim_start_matches('\u{feff}');
    let mut mesh = MeshBuilder::default();
    let mut tools = HashMap::<u32, f32>::new();
    let mut drill_templates = HashMap::<u32, Geometry>::new();
    let mut tool = 0;
    let mut unit = 25.4;
    let mut format = Format {
        integer: 2,
        decimal: 4,
        trailing: false,
    };
    let mut position = [0.0; 2];
    let mut route = false;
    let mut down = false;
    let mut interpolation = 0;
    let mut absolute = true;
    let mut warnings = Vec::new();
    let mut count = 0;
    let mut explicit_format = false;
    let mut guessed_format = false;
    let mut contextual_integer = false;
    let mut contextual_format = nc_hint.is_some();
    if let Some(nc) = nc_hint {
        unit = nc.unit_mm;
        format.integer = nc.integer;
        format.decimal = nc.decimal;
        format.trailing = nc.trailing_zeros_suppressed;
    }
    // Older Allegro exports have no M48 or TnC definitions. Their explicit
    // Holesize table is followed by one coordinate block per tool, split by M00.
    let mut legacy_tools = Vec::new();
    if !source.lines().any(|line| {
        let line = line.trim();
        line.starts_with('T') && fields(line).iter().any(|(k, _)| *k == b'C')
    }) {
        for line in source.lines() {
            let Some(record) = line.trim().strip_prefix(';').map(str::trim) else {
                continue;
            };
            let Some(record) = record.strip_prefix("Holesize ") else {
                continue;
            };
            let (id, rest) = record
                .split_once(". = ")
                .ok_or("Invalid Cadence tool table")?;
            let id: u32 = id.parse().map_err(|_| "Invalid Cadence tool number")?;
            if id != legacy_tools.len() as u32 + 1 {
                return Err("Cadence Holesize table must have consecutive tool numbers".into());
            }
            let diameter =
                parse_number(rest.split_whitespace().next().ok_or("Missing hole size")?)?;
            let diameter = if rest.split_whitespace().any(|w| w == "MILS") {
                diameter * 0.0254
            } else if rest.split_whitespace().any(|w| w == "MM") {
                diameter
            } else {
                return Err("Cadence Holesize table must declare MILS or MM".into());
            };
            if diameter <= 0.0 {
                return Err("Drill diameter must be positive".into());
            }
            legacy_tools.push(id);
            tools.insert(id, diameter);
        }
    }
    let implicit_tools =
        !legacy_tools.is_empty() && !source.lines().any(|line| line.trim().starts_with('T'));
    let mut legacy_tool_index = 0;
    if implicit_tools {
        tool = legacy_tools[0];
    }
    let mut header = false;
    let mut ended = false;
    for line in source.lines().map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(value) = line.strip_prefix(";FILE_FORMAT=") {
            if let Some((i, d)) = value.split_once(':') {
                format.integer = i.parse().map_err(|_| "Invalid drill format")?;
                format.decimal = d.parse().map_err(|_| "Invalid drill format")?;
                if !(1..=9).contains(&format.integer) || !(1..=9).contains(&format.decimal) {
                    return Err("Drill format digits must be between 1 and 9".into());
                }
                explicit_format = true;
            }
            continue;
        }
        if line == "M48" {
            header = true;
            continue;
        }
        if line.starts_with(';') || line == "%" {
            continue;
        }
        if line.starts_with("METRIC") || line.starts_with("INCH") || line == "M71" || line == "M72"
        {
            let metric = line.starts_with("METRIC") || line == "M71";
            unit = if metric { 1.0 } else { 25.4 };
            if !explicit_format && !contextual_format {
                format.integer = if metric { 3 } else { 2 };
                format.decimal = if metric { 3 } else { 4 };
            }
            // Excellon's LZ/TZ name the zeros *kept*, unlike Gerber's FS flags.
            if !contextual_format || line.contains("LZ") || line.contains("TZ") {
                format.trailing = line.contains("LZ");
            }
            if contextual_format && nc_hint.is_some_and(|nc| nc.unit_mm != unit) {
                contextual_format = false;
                format.trailing = line.contains("LZ");
                if !explicit_format {
                    format.integer = if metric { 3 } else { 2 };
                    format.decimal = if metric { 3 } else { 4 };
                }
                warnings.push(
                    "NC export settings disagree with drill units; settings were not applied"
                        .into(),
                );
            }
            if let Some(template) = line.split(',').find(|s| s.contains('.'))
                && let Some((i, d)) = template.split_once('.')
            {
                format.integer = i.len();
                format.decimal = d.len();
                explicit_format = true;
            }
            contextual_integer = false;
            // Only the integer width can be shared safely: drill precision often
            // differs from Gerber precision. Never guess TZ decimals from a film.
            if !explicit_format
                && !contextual_format
                && format.trailing
                && let Some(hint) = hint
                && hint.unit_mm == unit
                && (1..=9).contains(&hint.integer)
            {
                format.integer = hint.integer;
                contextual_integer = true;
            }
            continue;
        }
        if line.starts_with("M30") {
            ended = true;
            break;
        }
        if line == "M00" {
            if implicit_tools {
                legacy_tool_index += 1;
                tool = *legacy_tools
                    .get(legacy_tool_index)
                    .ok_or("Cadence drill has more tool blocks than its Holesize table")?;
            }
            continue;
        }
        if line == "M15" {
            down = true;
            continue;
        }
        if line == "M16" || line == "M17" {
            down = false;
            continue;
        }
        if line == "G90" {
            absolute = true;
            continue;
        }
        if line == "G91" {
            absolute = false;
            continue;
        }
        if line == "ICI,ON" {
            absolute = false;
            continue;
        }
        if line == "ICI,OFF" {
            absolute = true;
            continue;
        }
        if line == "G05" {
            route = false;
            continue;
        }
        if matches!(line, "M95" | "FMAT,2" | "VER,1" | "VER,2" | "G40") {
            continue;
        }
        let words = fields(line);
        if words.len() == 1 && matches!(words[0].0, b'F' | b'S') {
            parse_number(words[0].1)?; // Feed / spindle settings do not change geometry.
            continue;
        }
        if line.starts_with('T') {
            if line.contains('C') && !words.iter().any(|(k, _)| *k == b'C') {
                return Err("Missing or invalid drill diameter".into());
            }
            let id = words
                .iter()
                .find(|(k, _)| *k == b'T')
                .ok_or("Invalid drill tool")?
                .1
                .parse()
                .map_err(|_| "Invalid drill tool")?;
            if let Some((_, d)) = words.iter().find(|(k, _)| *k == b'C') {
                let diameter = parse_number(d)? * unit;
                if diameter <= 0.0 {
                    return Err("Drill diameter must be positive".into());
                }
                tools.insert(id, diameter);
            } else {
                tool = id;
            }
            continue;
        }
        for &(k, v) in &words {
            if k == b'G'
                && let Ok(code) = v.parse::<u32>()
            {
                match code {
                    0..=3 => {
                        route = true;
                        interpolation = code;
                    }
                    5 => route = false,
                    90 => absolute = true,
                    91 => absolute = false,
                    85 => {}
                    _ => {
                        if warnings.len() < 100 {
                            warnings.push(format!("Unsupported drill G-code G{code}"));
                        }
                    }
                }
            }
        }
        let target_for = |words: &[(u8, &str)], start: Point| -> Result<Point, String> {
            let mut p = start;
            for &(k, v) in words {
                if k == b'X' || k == b'Y' {
                    let axis = usize::from(k == b'Y');
                    p[axis] =
                        format.coordinate(v, unit)? + if absolute { 0.0 } else { start[axis] };
                }
            }
            Ok(p)
        };
        if !words.iter().any(|(k, _)| *k == b'X' || *k == b'Y') {
            if !line.starts_with('G') && !matches!(line, "M47" | "M49") && warnings.len() < 100 {
                warnings.push(format!("Unsupported drill command: {line}"));
            }
            continue;
        }
        if !explicit_format
            && !contextual_format
            && !contextual_integer
            && !guessed_format
            && words.iter().any(|(k, v)| {
                matches!(k, b'X' | b'Y')
                    && !v.contains('.')
                    && v.parse::<f32>().is_ok_and(|v| v != 0.0)
            })
        {
            warnings.push(if format.trailing {
                format!("Drill integer width not declared; assuming {} integer digits with variable decimal precision ({} LZ). Verify alignment.",
                    format.integer,if unit == 1.0 { "metric" } else { "inch" })
            } else {
                format!("Drill coordinate format not declared; assuming {}:{} {}. Verify alignment.",
                    format.integer,format.decimal,if unit == 1.0 { "metric" } else { "inch" })
            });
            guessed_format = true;
        }
        count += 1;
        if count > 2_000_000 {
            return Err("Safety limit: more than two million drills".into());
        }
        let diameter = *tools
            .get(&tool)
            .ok_or_else(|| format!("Drill tool T{tool} used before definition"))?;
        let label = format!("Drill · T{tool} · Ø{diameter:.3} mm");
        if line.starts_with('R') {
            let repeat: usize = words
                .iter()
                .find(|(k, _)| *k == b'R')
                .ok_or("Invalid repeat hole")?
                .1
                .parse()
                .map_err(|_| "Invalid repeat hole count")?;
            if repeat > 10_000 {
                return Err("Repeat hole count exceeds 10,000".into());
            }
            let mut offset = [0.0; 2];
            for &(k, v) in &words {
                if k == b'X' || k == b'Y' {
                    offset[usize::from(k == b'Y')] = format.coordinate(v, unit)?;
                }
            }
            for _ in 0..repeat {
                position[0] += offset[0];
                position[1] += offset[1];
                flash_drill(&mut mesh, &mut drill_templates, position, diameter, &label)?;
            }
            count += repeat;
            continue;
        }
        if let Some((start, end)) = line.split_once("G85") {
            let a = target_for(&fields(start), position)?;
            let b = target_for(&fields(end), a)?;
            mesh.stroke_slot(a, b, diameter, &label)?;
            position = b;
        } else {
            let target = target_for(&words, position)?;
            if route {
                if down && interpolation > 0 {
                    let path = if interpolation >= 2 {
                        let mut offset = [0.0; 2];
                        for &(k, v) in &words {
                            if k == b'I' || k == b'J' {
                                offset[usize::from(k == b'J')] = format.coordinate(v, unit)?;
                            }
                        }
                        arc_points(position, target, offset, interpolation == 2, false)
                    } else {
                        vec![position, target]
                    };
                    mesh.stroke(&path, diameter, true, &label)?;
                }
            } else {
                flash_drill(&mut mesh, &mut drill_templates, target, diameter, &label)?;
            }
            position = target;
        }
    }
    if tools.is_empty() && !(header && ended && tool == 0 && count == 0 && warnings.is_empty()) {
        return Err("No Excellon tool definitions found".into());
    }
    Ok(Parsed {
        geometry: mesh.finish(),
        warnings,
        file_function: Some("Drill".into()),
        apertures: tools.len(),
    })
}

fn flash_drill(
    mesh: &mut MeshBuilder,
    templates: &mut HashMap<u32, Geometry>,
    point: Point,
    diameter: f32,
    label: &str,
) -> Result<(), String> {
    let key = diameter.to_bits();
    if let std::collections::hash_map::Entry::Vacant(entry) = templates.entry(key) {
        let mut template = MeshBuilder::default();
        template.fill_circle([0.0; 2], diameter, 0.0, true, "")?;
        entry.insert(template.finish());
    }
    mesh.append(&templates[&key], point, true, label)
}
