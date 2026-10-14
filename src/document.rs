use crate::geometry::{Bounds, Geometry, Point};
use crate::parser::{self, Parsed};
use crate::snapping::{SnapHit, SnapMode, distance_squared};
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;
use std::collections::HashMap;
use std::io::{Cursor, Read, Seek};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use web_time::{Duration, Instant};

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum LayerKind {
    BottomCopper,
    InnerCopper,
    Copper,
    Mask,
    Paste,
    Silk,
    BottomSilk,
    Outline,
    Drill,
    Assembly,
    Drawing,
    Other,
}

impl LayerKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::BottomCopper => "Bottom copper",
            Self::InnerCopper => "Inner copper",
            Self::Copper => "Top copper",
            Self::Mask => "Solder mask",
            Self::Paste => "Solder paste",
            Self::Silk => "Silkscreen",
            Self::BottomSilk => "Bottom silkscreen",
            Self::Outline => "Board outline",
            Self::Drill => "Drill / route",
            Self::Assembly => "Assembly drawing",
            Self::Drawing => "Fabrication drawing",
            Self::Other => "Artwork",
        }
    }
    pub fn color(self) -> [u8; 3] {
        match self {
            Self::Copper => [242, 160, 105],
            Self::BottomCopper => [73, 190, 184],
            Self::InnerCopper => [161, 146, 218],
            Self::Silk | Self::BottomSilk => [220, 228, 218],
            Self::Mask => [117, 173, 139],
            Self::Paste => [178, 155, 220],
            Self::Outline => [130, 153, 169],
            Self::Drill => [10, 17, 22],
            Self::Assembly => [174, 191, 217],
            Self::Drawing => [146, 161, 177],
            Self::Other => [133, 163, 221],
        }
    }

    pub fn default_visible(self) -> bool {
        !matches!(
            self,
            Self::Mask
                | Self::Paste
                | Self::InnerCopper
                | Self::BottomSilk
                | Self::Assembly
                | Self::Drawing
        )
    }

    pub fn default_opacity(self) -> f32 {
        if self == Self::BottomCopper {
            0.45
        } else {
            1.0
        }
    }

    fn is_board_geometry(self) -> bool {
        matches!(
            self,
            Self::Copper | Self::BottomCopper | Self::InnerCopper | Self::Outline | Self::Drill
        )
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Layer {
    pub name: String,
    pub kind: LayerKind,
    pub geometry: Arc<Geometry>,
    pub visible: bool,
    pub color: [u8; 3],
    pub opacity: f32,
    pub warnings: Vec<String>,
    pub apertures: usize,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Document {
    pub name: String,
    pub layers: Vec<Layer>,
    pub bounds: Bounds,
    pub load_time: Duration,
    pub errors: Vec<String>,
    pub demo: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DocumentSnap {
    pub layer: usize,
    pub hit: SnapHit,
}

impl Document {
    /// Measure only visible, non-transparent layers. Prefer the selected layer
    /// when targets occupy exactly the same position, not when it is farther away.
    pub fn snap(
        &self,
        point: Point,
        radius: f32,
        mode: SnapMode,
        selected: usize,
    ) -> Option<DocumentSnap> {
        self.layers
            .iter()
            .enumerate()
            .filter(|(_, layer)| layer.visible && layer.opacity > 0.0)
            .filter_map(|(layer, data)| {
                data.geometry
                    .snap(point, radius, mode)
                    .map(|hit| DocumentSnap { layer, hit })
            })
            .min_by(|a, b| {
                a.hit
                    .kind
                    .cmp(&b.hit.kind)
                    .then(
                        distance_squared(a.hit.point, point)
                            .total_cmp(&distance_squared(b.hit.point, point)),
                    )
                    .then((a.layer != selected).cmp(&(b.layer != selected)))
                    .then(b.layer.cmp(&a.layer))
            })
    }

    pub fn primitives(&self) -> usize {
        self.layers.iter().map(|l| l.geometry.features.len()).sum()
    }
    pub fn triangles(&self) -> usize {
        self.layers
            .iter()
            .map(|l| l.geometry.indices.len() / 3)
            .sum()
    }
    pub fn memory_bytes(&self) -> usize {
        self.layers
            .iter()
            .map(|l| l.geometry.vertices.len() * 8 + l.geometry.indices.len() * 4)
            .sum()
    }
    pub fn warning_count(&self) -> usize {
        self.errors.len() + self.layers.iter().map(|l| l.warnings.len()).sum::<usize>()
    }

    /// Fabrication/assembly sheets can contain remote title blocks. Fit the
    /// displayed layers, not every hidden annotation in the archive.
    pub fn visible_bounds(&self) -> Bounds {
        let mut bounds = Bounds::default();
        for layer in &self.layers {
            if layer.visible && layer.opacity > 0.0 {
                bounds.union(layer.geometry.bounds);
            }
        }
        if bounds.valid() { bounds } else { self.bounds }
    }
}

fn name_kind(name: &str) -> LayerKind {
    let n = Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(name)
        .to_ascii_lowercase();
    let stem = Path::new(&n)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(&n)
        .trim_start_matches(|c: char| c.is_ascii_digit() || matches!(c, '_' | '-' | '.' | ' '));
    let tokens: Vec<_> = stem.split(['_', '-', '.', ' ']).collect();
    if stem.contains("drilldwg") || stem.contains("drill_drawing") || stem.contains("drillmap") {
        LayerKind::Drawing
    } else if stem.contains("assy") || stem.contains("assembly") {
        LayerKind::Assembly
    } else if n.ends_with(".gpb")
        || n.ends_with(".gpt")
        || Path::new(&n)
            .extension()
            .and_then(|e| e.to_str())
            .and_then(|e| e.strip_prefix("gm"))
            .is_some_and(|s| s != "1" && !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
    {
        // Mechanical-layer numbers are user-defined. Do not invent assembly
        // semantics for GM14/GM15; retain them as hidden fabrication drawings.
        LayerKind::Drawing
    } else if n.ends_with(".drl")
        || n.ends_with(".drd")
        || n.ends_with(".xln")
        || n.ends_with(".exc")
        || n.ends_with(".rou")
    {
        LayerKind::Drill
    } else if n.contains("edge_cuts")
        || n.contains("outline")
        || n.ends_with(".gko")
        || n.ends_with(".gm1")
    {
        LayerKind::Outline
    } else if stem == "ssb"
        || n.ends_with(".gbo")
        || (n.contains("silk") && (tokens.contains(&"bottom") || tokens.contains(&"b")))
    {
        LayerKind::BottomSilk
    } else if stem == "sst" || n.contains("silk") || n.ends_with(".gto") {
        LayerKind::Silk
    } else if matches!(stem, "smt" | "smb")
        || n.contains("mask")
        || n.ends_with(".gts")
        || n.ends_with(".gbs")
    {
        LayerKind::Mask
    } else if matches!(stem, "spt" | "spb")
        || n.contains("paste")
        || n.ends_with(".gtp")
        || n.ends_with(".gbp")
    {
        LayerKind::Paste
    } else if tokens.contains(&"bottom") || n.ends_with(".gbl") || n.contains("b_cu") {
        LayerKind::BottomCopper
    } else if Path::new(&n)
        .extension()
        .and_then(|e| e.to_str())
        .and_then(|e| e.strip_prefix('g'))
        .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        || tokens.iter().any(|t| {
            t.strip_prefix('l')
                .or_else(|| t.strip_prefix("in"))
                .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        })
    {
        LayerKind::InnerCopper
    } else if tokens.contains(&"top") || n.ends_with(".gtl") || n.contains("f_cu") {
        LayerKind::Copper
    } else {
        LayerKind::Other
    }
}

fn function_kind(function: &str) -> Option<LayerKind> {
    let f = function.to_ascii_lowercase();
    Some(if f.contains("assembly") {
        LayerKind::Assembly
    } else if f.contains("drawing") || f.contains("drillmap") {
        LayerKind::Drawing
    } else if f.contains("drill") {
        LayerKind::Drill
    } else if f.contains("profile") {
        LayerKind::Outline
    } else if f.contains("legend") {
        if f.contains("bot") {
            LayerKind::BottomSilk
        } else {
            LayerKind::Silk
        }
    } else if f.contains("soldermask") {
        LayerKind::Mask
    } else if f.contains("paste") {
        LayerKind::Paste
    } else if f.contains("copper") {
        if f.contains("bot") {
            LayerKind::BottomCopper
        } else if f.contains("inr") || f.contains("inner") {
            LayerKind::InnerCopper
        } else {
            LayerKind::Copper
        }
    } else {
        return None;
    })
}

fn legacy_kind(source: &str) -> Option<LayerKind> {
    let mut layer_names = Vec::new();
    let mut film = None;
    for line in source.lines().take(120) {
        let Some(comment) = line
            .trim()
            .strip_prefix("G04")
            .or_else(|| line.trim().strip_prefix("G4 "))
        else {
            continue;
        };
        let Some((key, value)) = comment.trim().trim_end_matches('*').split_once(':') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("Layer") {
            layer_names.push(value.trim().to_ascii_lowercase());
        } else if key.trim().eq_ignore_ascii_case("Film Name") {
            film = Some(value.trim());
        }
    }
    // These sheets may also contain OUTLINE or SILKSCREEN sublayers; their primary
    // function must win so title blocks/assembly fills don't cover the actual PCB.
    if layer_names.iter().any(|s| s.contains("nclegend")) {
        return Some(LayerKind::Drawing);
    }
    if layer_names.iter().any(|s| s.contains("assembly")) {
        return Some(LayerKind::Assembly);
    }
    if let Some(layer) = layer_names.iter().find_map(|s| s.strip_prefix("etch/")) {
        return Some(match layer {
            "top" => LayerKind::Copper,
            "bottom" => LayerKind::BottomCopper,
            _ => LayerKind::InnerCopper,
        });
    }
    if layer_names.iter().any(|s| s.contains("soldermask")) {
        return Some(LayerKind::Mask);
    }
    if layer_names
        .iter()
        .any(|s| s.contains("pastemask") || s.contains("solderpaste"))
    {
        return Some(LayerKind::Paste);
    }
    if let Some(layer) = layer_names.iter().find(|s| s.contains("silkscreen")) {
        return Some(if layer.contains("bottom") {
            LayerKind::BottomSilk
        } else {
            LayerKind::Silk
        });
    }
    if layer_names
        .iter()
        .any(|s| s.starts_with("board geometry/") && s.contains("outline"))
    {
        return Some(LayerKind::Outline);
    }
    film.map(name_kind).filter(|kind| *kind != LayerKind::Other)
}

fn classify(name: &str, function: Option<&str>, legacy: Option<LayerKind>) -> LayerKind {
    if let Some(kind) = function.and_then(function_kind) {
        return kind;
    }
    let named = name_kind(name);
    if matches!(named, LayerKind::Drawing | LayerKind::Assembly) {
        return named;
    }
    legacy.unwrap_or(named)
}

fn nc_settings(name: &str) -> bool {
    Path::new(name)
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("nc_param.txt"))
}

fn auxiliary_text(name: &str, source: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("txt"))
        && !source
            .trim_start_matches('\u{feff}')
            .lines()
            .take(120)
            .any(|line| {
                let line = line.trim();
                line == "M48"
                    || line.starts_with("INCH")
                    || line.starts_with("METRIC")
                    || line.starts_with("%FS")
                    || line.starts_with("%MO")
                    || line.starts_with("%AD")
                    || line.starts_with("G04")
                    || line.starts_with("G4 ")
                    || line.starts_with(";FILE_FORMAT=")
                    || line.trim_start_matches(';').trim().starts_with("Holesize ")
            })
}

pub fn from_sources(
    name: String,
    sources: Vec<(String, String)>,
    demo: bool,
) -> Result<Document, String> {
    let start = Instant::now();
    let mut hints = HashMap::<PathBuf, Option<parser::GerberFormatHint>>::new();
    let mut nc_hints = HashMap::<PathBuf, Option<parser::DrillFormatHint>>::new();
    let mut errors = Vec::new();
    for (name, source) in &sources {
        if nc_settings(name) {
            let parent = Path::new(name)
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .to_owned();
            if let Some(hint) = parser::cadence_nc_format(source) {
                nc_hints.entry(parent).and_modify(|existing| {
                    if existing.is_some_and(|value| value != hint) {
                        *existing = None;
                        errors.push(format!("{name}: Conflicting NC export settings; drill format remains ambiguous"));
                    }
                }).or_insert(Some(hint));
            } else {
                errors.push(format!("{name}: Invalid Cadence NC export settings"));
            }
            continue;
        }
        if let Some(hint) = parser::gerber_format_hint(source) {
            let parent = Path::new(name)
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .to_owned();
            hints
                .entry(parent)
                .and_modify(|existing| {
                    if existing.is_some_and(|value| {
                        value.integer != hint.integer || value.unit_mm != hint.unit_mm
                    }) {
                        *existing = None;
                    }
                })
                .or_insert(Some(hint));
        }
    }
    let sources: Vec<_> = sources
        .into_iter()
        .filter(|(name, source)| !nc_settings(name) && !auxiliary_text(name, source))
        .collect();
    #[cfg(not(target_arch = "wasm32"))]
    let sources = sources.into_par_iter();
    #[cfg(target_arch = "wasm32")]
    let sources = sources.into_iter();
    let results: Vec<_> = sources
        .map(|(name, source)| {
            let extension = Path::new(&name)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let drill = source.trim_start_matches('\u{feff}').lines().any(|l| {
                let l = l.trim();
                l == "M48"
                    || l.starts_with("INCH")
                    || l.starts_with("METRIC")
                    || l.strip_prefix(';')
                        .is_some_and(|s| s.trim().starts_with("Holesize "))
            }) || matches!(extension.as_str(), "drl" | "drd" | "xln" | "exc" | "rou");
            let result = if drill {
                let hint = hints
                    .get(Path::new(&name).parent().unwrap_or_else(|| Path::new("")))
                    .copied()
                    .flatten();
                let nc = nc_hints
                    .get(Path::new(&name).parent().unwrap_or_else(|| Path::new("")))
                    .copied()
                    .flatten();
                parser::parse_excellon_with_hints(&source, hint, nc)
            } else {
                parser::parse_gerber(&source)
            };
            let legacy = if drill { None } else { legacy_kind(&source) };
            (name, result, legacy)
        })
        .collect();
    let mut layers = Vec::new();
    let mut bounds = Bounds::default();
    for (name, result, legacy) in results {
        match result {
            Ok(Parsed {
                geometry,
                warnings,
                file_function,
                apertures,
            }) => {
                let kind = classify(&name, file_function.as_deref(), legacy);
                bounds.union(geometry.bounds);
                layers.push(Layer {
                    name,
                    kind,
                    geometry: Arc::new(geometry),
                    visible: kind.default_visible(),
                    color: kind.color(),
                    opacity: kind.default_opacity(),
                    warnings,
                    apertures,
                });
            }
            Err(e) => errors.push(format!("{name}: {e}")),
        }
    }
    if layers.is_empty() || !bounds.valid() {
        return Err(if errors.is_empty() {
            "No drawable geometry found".into()
        } else {
            errors.join("\n")
        });
    }
    layers.sort_by_key(|layer| layer.kind);
    let mut board = Bounds::default();
    for layer in &layers {
        if layer.kind.is_board_geometry() {
            board.union(layer.geometry.bounds);
        }
    }
    if board.valid() {
        bounds = board;
    }
    Ok(Document {
        name,
        layers,
        bounds,
        load_time: start.elapsed(),
        errors,
        demo,
    })
}

pub fn supported_path(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "gbr"
            | "gbx"
            | "art"
            | "pho"
            | "ger"
            | "gerber"
            | "gtl"
            | "gbl"
            | "gto"
            | "gbo"
            | "gts"
            | "gbs"
            | "gtp"
            | "gbp"
            | "gpb"
            | "gpt"
            | "gko"
            | "gm1"
            | "gm2"
            | "gml"
            | "drl"
            | "drd"
            | "xln"
            | "exc"
            | "rou"
            | "txt"
    ) || ext
        .strip_prefix("gm")
        .or_else(|| ext.strip_prefix('g'))
        .is_some_and(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(not(target_arch = "wasm32"))]
fn collect_paths(path: &Path, files: &mut Vec<PathBuf>, depth: usize) -> Result<(), String> {
    if files.len() >= 256 {
        return Err("A workspace can contain at most 256 files".into());
    }
    if path.is_dir() {
        if depth > 8 {
            return Err("Directory nesting exceeds eight levels".into());
        }
        for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_type().map_err(|e| e.to_string())?.is_symlink() {
                continue;
            }
            collect_paths(&entry.path(), files, depth + 1)?;
        }
    } else if depth == 0
        || supported_path(path)
        || path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("zip"))
    {
        files.push(path.to_owned());
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn load(paths: Vec<PathBuf>) -> Result<Document, String> {
    let start = Instant::now();
    let name = paths
        .first()
        .and_then(|p| {
            if p.is_dir() || paths.len() == 1 {
                p.file_stem()
            } else {
                p.parent()?.file_name()
            }
        })
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Untitled board".into());
    let mut files = Vec::new();
    for path in paths {
        collect_paths(&path, &mut files, 0)?;
    }
    files.sort();
    files.dedup();
    // Preserve directory identity so one board's nc_param.txt/format hints
    // cannot change another board's drill scale in a recursive folder import.
    let mut common = files
        .first()
        .and_then(|f| f.parent())
        .unwrap_or(Path::new(""))
        .to_owned();
    while files.iter().any(|file| !file.starts_with(&common)) {
        if !common.pop() {
            break;
        }
    }
    let mut sources = Vec::new();
    let mut total_bytes = 0_u64;
    for file in files {
        if file
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("zip"))
        {
            let reader =
                std::fs::File::open(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            let scope = file
                .strip_prefix(&common)
                .unwrap_or(&file)
                .to_string_lossy();
            read_zip(reader, &mut sources, &mut total_bytes, &scope)?;
        } else {
            let size = std::fs::metadata(&file).map_err(|e| e.to_string())?.len();
            total_bytes += size;
            if size > 128 * 1024 * 1024 || total_bytes > 512 * 1024 * 1024 {
                return Err("Input exceeds the 512 MiB workspace limit".into());
            }
            let source =
                std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            sources.push((
                file.strip_prefix(&common)
                    .unwrap_or(&file)
                    .to_string_lossy()
                    .into_owned(),
                source,
            ));
        }
    }
    if sources.is_empty() {
        return Err("No Gerber or Excellon files found in this selection".into());
    }
    if sources.len() > 256 {
        return Err("A workspace can contain at most 256 layers".into());
    }
    let mut document = from_sources(name, sources, false)?;
    document.load_time = start.elapsed();
    Ok(document)
}

fn read_zip(
    reader: impl Read + Seek,
    sources: &mut Vec<(String, String)>,
    total_bytes: &mut u64,
    scope: &str,
) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| format!("Invalid ZIP: {e}"))?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().map_err(|e| e.to_string())?.into_owned();
        if entry.is_dir()
            || !supported_path(Path::new(&name))
            || name
                .split('/')
                .any(|part| part == "__MACOSX" || part.starts_with("._"))
        {
            continue;
        }
        *total_bytes += entry.size();
        if entry.size() > 128 * 1024 * 1024
            || *total_bytes > 512 * 1024 * 1024
            || sources.len() >= 256
        {
            return Err("Archive exceeds file-count or uncompressed-size safety limits".into());
        }
        let mut data = Vec::new();
        entry
            .by_ref()
            .take(128 * 1024 * 1024 + 1)
            .read_to_end(&mut data)
            .map_err(|e| e.to_string())?;
        if data.len() > 128 * 1024 * 1024 {
            return Err("Archive entry exceeds 128 MiB".into());
        }
        sources.push((
            format!("{scope}/{name}"),
            String::from_utf8(data).map_err(|_| "Gerber files must be UTF-8/ASCII text")?,
        ));
    }
    Ok(())
}

/// Browser-friendly import: all source data stays in memory. ZIP entries are
/// parsed directly and use the same limits and classification as native imports.
pub fn load_bytes(files: Vec<(String, Vec<u8>)>) -> Result<Document, String> {
    let start = Instant::now();
    if files.is_empty() || files.len() > 256 {
        return Err("Select between 1 and 256 files".into());
    }
    let name = files
        .first()
        .and_then(|(name, _)| Path::new(name).file_stem())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Uploaded board".into());
    let mut sources = Vec::new();
    let mut total_bytes = 0;
    for (name, data) in files {
        let path = Path::new(&name);
        let is_zip = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("zip"));
        if path.components().count() > 1 && !supported_path(path) && !is_zip {
            continue;
        }
        if data.len() > 128 * 1024 * 1024 {
            return Err(format!("{name}: file exceeds 128 MiB"));
        }
        if is_zip {
            read_zip(Cursor::new(data), &mut sources, &mut total_bytes, &name)?;
        } else {
            total_bytes += data.len() as u64;
            if total_bytes > 512 * 1024 * 1024 || sources.len() >= 256 {
                return Err("Input exceeds workspace safety limits".into());
            }
            sources.push((
                name,
                String::from_utf8(data).map_err(|_| "Gerber files must be UTF-8/ASCII text")?,
            ));
        }
    }
    if sources.is_empty() {
        return Err("No Gerber or Excellon files found in this selection".into());
    }
    let mut doc = from_sources(name, sources, false)?;
    doc.load_time = start.elapsed();
    Ok(doc)
}

pub fn demo() -> Document {
    from_sources("Atlas · controller rev. 04".into(), demo_sources(), true)
        .expect("Built-in board is valid")
}

struct GerberWriter(String);
impl GerberWriter {
    fn new(function: &str) -> Self {
        Self(format!(
            "G04 Gerbrr example board*\n%FSLAX36Y36*%\n%MOMM*%\n%TF.FileFunction,{function}*%\n%ADD10C,0.3*%\n%ADD11C,1.8X0.8*%\n%ADD12R,1.4X0.6*%\n%ADD13R,1.8X1.1*%\n%ADD14C,0.65*%\n%ADD15C,3.4X2.2*%\n%ADD16C,0.15*%\nG75*\nD10*\n"
        ))
    }
    fn aperture(&mut self, id: u32) {
        self.0.push_str(&format!("D{id}*\n"));
    }
    fn xy(&mut self, p: Point, d: u32) {
        self.0.push_str(&format!(
            "X{}Y{}D{d:02}*\n",
            (p[0] * 1e6).round() as i64,
            (p[1] * 1e6).round() as i64
        ));
    }
    fn trace(&mut self, points: &[Point]) {
        self.xy(points[0], 2);
        for p in &points[1..] {
            self.xy(*p, 1);
        }
    }
    fn pad(&mut self, p: Point) {
        self.xy(p, 3);
    }
    fn finish(mut self) -> String {
        self.0.push_str("M02*\n");
        self.0
    }
}

/// Real RS-274X / Excellon source, also exportable with `--write-demo DIR`.
pub fn demo_sources() -> Vec<(String, String)> {
    let mut top = GerberWriter::new("Copper,L1,Top");
    let mut bottom = GerberWriter::new("Copper,L2,Bot");
    let mut silk = GerberWriter::new("Legend,Top");
    let mut outline = GerberWriter::new("Profile,NP");
    let mut mask = GerberWriter::new("Soldermask,Top");
    let mut drill =
        String::from("M48\n;FILE_FORMAT=3:3\nMETRIC,TZ\nT1C0.8\nT2C0.3\nT3C2.2\n%\nT1\n");
    top.aperture(11);
    mask.aperture(11);
    for side in [7.0, 73.0] {
        for i in 0..16 {
            let p = [side, 8.0 + i as f32 * 2.54];
            top.pad(p);
            mask.pad(p);
            drill.push_str(&format!("X{:.3}Y{:.3}\n", p[0], p[1]));
            silk.aperture(16);
            silk.trace(&[
                [side - 1.4, p[1] - 1.2],
                [side + 1.4, p[1] - 1.2],
                [side + 1.4, p[1] + 1.2],
            ]);
        }
    }
    top.aperture(12);
    for i in 0..16 {
        let t = 21.75 + i as f32 * 0.7;
        top.pad([31.0, t]);
        top.pad([49.0, t]);
        top.aperture(13); // small surrounding passives
        if i % 2 == 0 {
            top.pad([24.0, t]);
            top.pad([26.5, t]);
            top.pad([55.0, t]);
            top.pad([57.5, t]);
        }
        top.aperture(12);
    }
    // Vertical QFP pads use a rotated rectangular aperture.
    top.0.push_str("%LR90*%\n");
    for i in 0..16 {
        let t = 34.75 + i as f32 * 0.7;
        top.pad([t, 18.0]);
        top.pad([t, 36.0]);
    }
    top.0.push_str("%LR0*%\n");
    top.aperture(10);
    bottom.aperture(16);
    for i in 0..16 {
        let y = 21.75 + i as f32 * 0.7;
        let header = 8.0 + i as f32 * 2.54;
        let bend = 14.0 + i as f32 * 0.52;
        top.trace(&[[7.0, header], [bend, header], [bend + 3.0, y], [31.0, y]]);
        top.trace(&[
            [49.0, y],
            [60.0 - i as f32 * 0.48, y],
            [66.0 - i as f32 * 0.25, header],
            [73.0, header],
        ]);
        bottom.trace(&[
            [7.0, header],
            [11.0, header],
            [18.0 + i as f32 * 0.5, 14.0],
            [35.0 + i as f32 * 0.7, 14.0],
            [35.0 + i as f32 * 0.7, 18.0],
        ]);
        let t = 34.75 + i as f32 * 0.7;
        top.aperture(16);
        let destination_y = 49.0 - i as f32 * 0.3;
        top.trace(&[
            [t, 36.0],
            [t, 38.0],
            [t + destination_y - 38.0, destination_y],
            [65.0, destination_y],
        ]);
        top.aperture(10);
    }
    top.aperture(14);
    drill.push_str("T2\n");
    for i in 0..16 {
        for p in [
            [34.75 + i as f32 * 0.7, 14.0],
            [65.0, 49.0 - i as f32 * 0.3],
        ] {
            top.pad(p);
            drill.push_str(&format!("X{:.3}Y{:.3}\n", p[0], p[1]));
        }
    }
    // Power section, decoupling, test points, and an oscillator.
    top.aperture(13);
    for x in [18.0, 24.0, 30.0, 36.0, 42.0, 48.0, 54.0, 60.0] {
        for y in [8.0, 10.5] {
            top.pad([x, y]);
        }
        silk.aperture(16);
        silk.trace(&[
            [x - 1.3, 7.0],
            [x + 1.3, 7.0],
            [x + 1.3, 11.5],
            [x - 1.3, 11.5],
            [x - 1.3, 7.0],
        ]);
    }
    top.aperture(10);
    top.trace(&[[18.0, 8.0], [60.0, 8.0], [65.0, 13.0], [65.0, 40.0]]);
    silk.aperture(16);
    silk.trace(&[
        [32.5, 19.5],
        [47.5, 19.5],
        [47.5, 34.5],
        [32.5, 34.5],
        [32.5, 19.5],
    ]);
    silk.trace(&[[34.0, 20.5], [35.0, 20.5], [34.0, 21.5], [34.0, 20.5]]);
    draw_text(&mut silk, "ATLAS", [18.0, 44.0], 2.4);
    draw_text(&mut silk, "04", [18.0, 40.0], 1.5);
    draw_text(&mut silk, "U1", [37.0, 25.5], 1.6);
    draw_text(&mut silk, "GERBRR", [28.2, 3.0], 1.2);
    outline.aperture(10);
    outline.trace(&[[3.0, 0.0], [77.0, 0.0]]);
    outline.0.push_str("G03*\nX80000000Y3000000I0J3000000D01*\nG01*\nX80000000Y51000000D01*\nG03*\nX77000000Y54000000I-3000000J0D01*\nG01*\nX3000000Y54000000D01*\nG03*\nX0Y51000000I0J-3000000D01*\nG01*\nX0Y3000000D01*\nG03*\nX3000000Y0I3000000J0D01*\nG01*\n");
    top.aperture(15);
    drill.push_str("T3\n");
    for p in [[3.5, 3.5], [76.5, 3.5], [3.5, 50.5], [76.5, 50.5]] {
        top.pad(p);
        drill.push_str(&format!("X{:.3}Y{:.3}\n", p[0], p[1]));
    }
    drill.push_str("M30\n");
    vec![
        ("atlas-F_Cu.gtl".into(), top.finish()),
        ("atlas-B_Cu.gbl".into(), bottom.finish()),
        ("atlas-F_Silkscreen.gto".into(), silk.finish()),
        ("atlas-Edge_Cuts.gko".into(), outline.finish()),
        ("atlas-F_Mask.gts".into(), mask.finish()),
        ("atlas-PTH.drl".into(), drill),
    ]
}

fn draw_text(w: &mut GerberWriter, text: &str, origin: Point, height: f32) {
    // A compact single-stroke engineering alphabet, encoded on a 2×3 grid.
    for (index, c) in text.chars().enumerate() {
        let strokes: &[&[(f32, f32)]] = match c {
            'G' => &[&[(2., 3.), (0., 3.), (0., 0.), (2., 0.), (2., 1.5), (1., 1.5)]],
            'B' => &[
                &[
                    (0., 0.),
                    (0., 3.),
                    (1.6, 3.),
                    (2., 2.6),
                    (2., 1.8),
                    (1.6, 1.5),
                    (0., 1.5),
                ],
                &[(1.6, 1.5), (2., 1.2), (2., 0.4), (1.6, 0.), (0., 0.)],
            ],
            'A' => &[
                &[(0., 0.), (0., 2.), (1., 3.), (2., 2.), (2., 0.)],
                &[(0., 1.5), (2., 1.5)],
            ],
            'T' => &[&[(0., 3.), (2., 3.)], &[(1., 3.), (1., 0.)]],
            'L' => &[&[(0., 3.), (0., 0.), (2., 0.)]],
            'S' => &[&[(2., 3.), (0., 3.), (0., 1.5), (2., 1.5), (2., 0.), (0., 0.)]],
            'U' => &[&[(0., 3.), (0., 0.), (2., 0.), (2., 3.)]],
            'P' => &[&[(0., 0.), (0., 3.), (2., 3.), (2., 1.5), (0., 1.5)]],
            'E' => &[
                &[(2., 3.), (0., 3.), (0., 0.), (2., 0.)],
                &[(0., 1.5), (1.6, 1.5)],
            ],
            'R' => &[&[(0., 0.), (0., 3.), (2., 3.), (2., 1.5), (0., 1.5), (2., 0.)]],
            '0' => &[&[(0., 0.), (0., 3.), (2., 3.), (2., 0.), (0., 0.)]],
            '4' => &[&[(0., 3.), (0., 1.5), (2., 1.5)], &[(2., 3.), (2., 0.)]],
            '1' => &[&[(0.5, 2.5), (1., 3.), (1., 0.)]],
            _ => &[],
        };
        for stroke in strokes {
            let points: Vec<_> = stroke
                .iter()
                .map(|(x, y)| {
                    [
                        origin[0] + (index as f32 * 3.0 + x) * height / 3.0,
                        origin[1] + y * height / 3.0,
                    ]
                })
                .collect();
            w.trace(&points);
        }
    }
}
