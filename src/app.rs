use crate::document::{self, Document};
use crate::geometry::Point;
use crate::renderer::{BoardCallback, Camera, RenderLayer, Renderer};
use crate::snapping::{SnapKind, SnapMode};
use eframe::egui::{
    self, Align, Align2, Color32, FontId, Key, Margin, Pos2, Rect, RichText, Sense, Stroke, Vec2,
};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use web_time::{Duration, Instant};

const BG: Color32 = Color32::from_rgb(13, 19, 25);
const PANEL: Color32 = Color32::from_rgb(20, 27, 34);
const CARD: Color32 = Color32::from_rgb(27, 35, 43);
const LINE: Color32 = Color32::from_rgb(43, 53, 63);
const TEXT: Color32 = Color32::from_rgb(221, 230, 237);
const MUTED: Color32 = Color32::from_rgb(133, 149, 161);
const ACCENT: Color32 = Color32::from_rgb(144, 219, 190);
const AMBER: Color32 = Color32::from_rgb(242, 177, 112);

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Pan,
    Inspect,
    Measure,
}

#[derive(Clone, Copy)]
pub(crate) struct MeasurementPoint {
    pub point: Point,
    pub snap: Option<document::DocumentSnap>,
}

enum Message {
    Loaded(Result<Document, String>),
    #[cfg(not(target_arch = "wasm32"))]
    Picked(Vec<PathBuf>),
    #[cfg(not(target_arch = "wasm32"))]
    Capture(PathBuf),
    #[cfg(not(target_arch = "wasm32"))]
    Error(String),
}

pub struct App {
    mascot: egui::TextureHandle,
    github_mark: egui::TextureHandle,
    document: Document,
    camera: Camera,
    generation: u64,
    selected: usize,
    scroll_to_selected: bool,
    sidebar_collapsed: bool,
    tool: Tool,
    grid: bool,
    substrate: bool,
    fit_pending: bool,
    canvas: Rect,
    pointer: Option<Point>,
    measurement: Vec<MeasurementPoint>,
    snap_enabled: bool,
    snap_mode: SnapMode,
    snap_hover: Option<document::DocumentSnap>,
    feature: Option<(usize, usize)>,
    receiver: Receiver<Message>,
    sender: mpsc::Sender<Message>,
    busy: bool,
    error: Option<String>,
    diagnostics: bool,
    help: bool,
    gpu: String,
    cpu_time: f32,
    solo_restore: Option<Vec<bool>>,
    capture: Option<PathBuf>,
    capture_frames: u8,
    #[cfg(target_arch = "wasm32")]
    ctx: egui::Context,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        paths: Vec<PathBuf>,
        capture: Option<PathBuf>,
    ) -> Self {
        theme(&cc.egui_ctx);
        let icon = crate::brand::icon();
        let mascot = cc.egui_ctx.load_texture(
            "gerbrr-mascot",
            egui::ColorImage::from_rgba_unmultiplied(
                [icon.width as usize, icon.height as usize],
                &icon.rgba,
            ),
            egui::TextureOptions::LINEAR,
        );
        let github = crate::brand::github_mark();
        let github_mark = cc.egui_ctx.load_texture(
            "github-mark",
            egui::ColorImage::from_rgba_unmultiplied(
                [github.width as usize, github.height as usize],
                &github.rgba,
            ),
            egui::TextureOptions::LINEAR,
        );
        let gpu = if let Some(state) = &cc.wgpu_render_state {
            state
                .renderer
                .write()
                .callback_resources
                .insert(Renderer::new(&state.device, state.target_format));
            let info = state.adapter.get_info();
            if info.name.is_empty() {
                format!("{:?}", info.backend)
            } else {
                info.name
            }
        } else {
            "Unavailable".into()
        };
        let (sender, receiver) = mpsc::channel();
        let mut app = Self {
            mascot,
            github_mark,
            document: document::demo(),
            camera: Camera::default(),
            generation: 0,
            selected: 0,
            scroll_to_selected: true,
            sidebar_collapsed: false,
            tool: Tool::Pan,
            grid: true,
            substrate: true,
            fit_pending: true,
            canvas: Rect::NOTHING,
            pointer: None,
            measurement: vec![],
            snap_enabled: true,
            snap_mode: SnapMode::Auto,
            snap_hover: None,
            feature: None,
            receiver,
            sender,
            busy: false,
            error: None,
            diagnostics: false,
            help: false,
            gpu,
            cpu_time: 0.0,
            solo_restore: None,
            capture,
            capture_frames: 0,
            #[cfg(target_arch = "wasm32")]
            ctx: cc.egui_ctx.clone(),
        };
        app.selected = app
            .document
            .layers
            .iter()
            .position(|l| l.kind == document::LayerKind::Copper)
            .unwrap_or(0);
        #[cfg(not(target_arch = "wasm32"))]
        if !paths.is_empty() {
            app.load(paths, &cc.egui_ctx);
        }
        #[cfg(target_arch = "wasm32")]
        let _ = paths;
        app
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn load(&mut self, paths: Vec<PathBuf>, ctx: &egui::Context) {
        if self.busy || paths.is_empty() {
            return;
        }
        self.busy = true;
        let sender = self.sender.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(|| document::load(paths)).unwrap_or_else(|_| {
                Err(
                    "A parser encountered an unexpected error; the current board is unchanged"
                        .into(),
                )
            });
            let _ = sender.send(Message::Loaded(result));
            ctx.request_repaint();
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn picker(&mut self, folder: bool, ctx: &egui::Context) {
        if self.busy {
            return;
        }
        let sender = self.sender.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let dialog = rfd::FileDialog::new().set_title(if folder {
                "Open board folder"
            } else {
                "Open Gerber layers or ZIP"
            });
            let paths = if folder {
                dialog.pick_folder().map(|p| vec![p])
            } else {
                dialog.pick_files()
            };
            if let Some(paths) = paths {
                let _ = sender.send(Message::Picked(paths));
                ctx.request_repaint();
            }
        });
    }

    #[cfg(target_arch = "wasm32")]
    fn picker(&mut self, folder: bool, _ctx: &egui::Context) {
        if !self.busy {
            crate::web::open_files(folder);
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn begin_browser_import(&mut self) -> bool {
        if self.busy {
            return false;
        }
        self.busy = true;
        self.error = None;
        self.ctx.request_repaint();
        true
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn complete_browser_import(&mut self, result: Result<Document, String>) {
        let _ = self.sender.send(Message::Loaded(result));
        self.ctx.request_repaint();
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn browser_document(&self) -> &Document {
        &self.document
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn browser_gpu(&self) -> &str {
        &self.gpu
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn browser_busy(&self) -> bool {
        self.busy
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn browser_measurement(&self) -> &[MeasurementPoint] {
        &self.measurement
    }
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn browser_measurement_preview(&self) -> Option<MeasurementPoint> {
        (self.tool == Tool::Measure)
            .then(|| {
                self.pointer.map(|point| MeasurementPoint {
                    point,
                    snap: self.snap_hover,
                })
            })
            .flatten()
    }
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn browser_measurement_view(&self) -> (Camera, Rect, bool, SnapMode) {
        (self.camera, self.canvas, self.snap_enabled, self.snap_mode)
    }

    fn accept(&mut self, document: Document) {
        self.document = document;
        self.generation += 1;
        self.selected = self
            .document
            .layers
            .iter()
            .position(|l| l.kind == document::LayerKind::Copper)
            .unwrap_or(0);
        self.fit_pending = true;
        self.scroll_to_selected = true;
        self.feature = None;
        self.measurement.clear();
        self.snap_hover = None;
        self.solo_restore = None;
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn export(&self, ctx: &egui::Context) {
        let sender = self.sender.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Save workspace image")
                .add_filter("PNG image", &["png"])
                .set_file_name("gerbrr-board.png")
                .save_file()
            {
                let _ = sender.send(Message::Capture(path));
                ctx.request_repaint();
            }
        });
    }

    #[cfg(target_arch = "wasm32")]
    fn export(&self, _ctx: &egui::Context) {
        crate::web::export_canvas();
    }

    fn solo(&mut self) {
        if let Some(restore) = self.solo_restore.take() {
            for (layer, visible) in self.document.layers.iter_mut().zip(restore) {
                layer.visible = visible;
            }
        } else {
            self.solo_restore = Some(self.document.layers.iter().map(|l| l.visible).collect());
            for (i, layer) in self.document.layers.iter_mut().enumerate() {
                layer.visible = i == self.selected;
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let open = ctx.input(|i| i.modifiers.command && i.key_pressed(Key::O));
        if open {
            self.picker(ctx.input(|i| i.modifiers.shift), ctx);
        }
        ctx.input(|i| {
            if i.key_pressed(Key::F) || i.key_pressed(Key::Home) {
                self.fit_pending = true;
            }
            if i.key_pressed(Key::G) {
                self.grid = !self.grid;
            }
            if i.key_pressed(Key::M) {
                self.tool = Tool::Measure;
                self.measurement.clear();
            }
            if self.tool == Tool::Measure && i.key_pressed(Key::N) && !i.modifiers.command {
                if i.modifiers.shift {
                    self.snap_mode = self.snap_mode.next();
                    self.snap_enabled = true;
                } else {
                    self.snap_enabled = !self.snap_enabled;
                }
            }
            if i.key_pressed(Key::V) {
                self.tool = Tool::Inspect;
            }
            if i.key_pressed(Key::H) {
                self.tool = Tool::Pan;
            }
            if i.key_pressed(Key::R) {
                self.camera.rotation = (self.camera.rotation + 90.0) % 360.0;
            }
            if i.key_pressed(Key::B) {
                self.camera.mirrored = !self.camera.mirrored;
            }
            if i.key_pressed(Key::S) {
                self.solo();
            }
            if i.key_pressed(Key::Escape) {
                self.measurement.clear();
                self.feature = None;
                self.tool = Tool::Pan;
            }
            if i.key_pressed(Key::Questionmark) {
                self.help = !self.help;
            }
            if self.canvas.is_positive() {
                if i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals) {
                    self.camera.zoom(1.25, self.canvas.center(), self.canvas);
                }
                if i.key_pressed(Key::Minus) {
                    self.camera.zoom(0.8, self.canvas.center(), self.canvas);
                }
            }
            for (index, key) in [
                Key::Num1,
                Key::Num2,
                Key::Num3,
                Key::Num4,
                Key::Num5,
                Key::Num6,
                Key::Num7,
                Key::Num8,
                Key::Num9,
            ]
            .into_iter()
            .enumerate()
            {
                if i.key_pressed(key)
                    && let Some(layer) = self.document.layers.get_mut(index)
                {
                    layer.visible = !layer.visible;
                }
            }
        });
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("header")
            .exact_size(66.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(Margin::symmetric(24, 12)),
            )
            .show(ui, |ui| {
                let wide = ui.available_width() > 1050.0;
                ui.horizontal_centered(|ui| {
                    ui.add(egui::Image::new((self.mascot.id(), Vec2::splat(42.0))))
                        .on_hover_text("Safety goggles on. Gerbers in.");
                    ui.add_space(7.0);
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(crate::brand::WORDMARK)
                                .size(21.0)
                                .strong()
                                .color(TEXT),
                        );
                        ui.label(RichText::new(crate::brand::TAGLINE).size(9.0).color(MUTED));
                    });
                    ui.add_space(if wide { 22.0 } else { 8.0 });
                    ui.separator();
                    ui.add_space(16.0);
                    if wide {
                        ui.label(RichText::new("Workspace").color(MUTED));
                        ui.label(RichText::new("/").color(LINE));
                    }
                    ui.add_sized(
                        [if wide { 225.0 } else { 130.0 }, 34.0],
                        egui::Label::new(RichText::new(&self.document.name).strong()).truncate(),
                    )
                    .on_hover_text(&self.document.name);
                    if self.document.demo {
                        pill(ui, "DEMO", ACCENT);
                    }
                    #[cfg(target_arch = "wasm32")]
                    if wide {
                        pill(ui, "WEB POC", ACCENT);
                    }
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("Open board").color(BG).strong())
                                    .fill(ACCENT)
                                    .min_size(Vec2::new(112.0, 34.0)),
                            )
                            .on_hover_text("Open files or ZIP · Ctrl+O")
                            .clicked()
                        {
                            self.picker(false, ui.ctx());
                        }
                        if ui
                            .button("Folder")
                            .on_hover_text("Open all layers in a folder · Ctrl+Shift+O")
                            .clicked()
                        {
                            self.picker(true, ui.ctx());
                        }
                        if ui
                            .button("Export")
                            .on_hover_text("Save the workspace as a PNG image")
                            .clicked()
                        {
                            self.export(ui.ctx());
                        }
                        if ui.button("?").on_hover_text("Keyboard shortcuts").clicked() {
                            self.help = !self.help;
                        }
                        let github = ui
                            .add(egui::Button::image(
                                egui::Image::new((self.github_mark.id(), Vec2::splat(16.0)))
                                    .tint(MUTED),
                            ))
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text("View source on GitHub · bndnsmth/gerbrr");
                        github.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                true,
                                "View Gerbrr on GitHub",
                            )
                        });
                        if github.clicked() {
                            ui.ctx()
                                .open_url(egui::OpenUrl::new_tab(crate::brand::REPOSITORY_URL));
                        }
                    });
                });
            });
    }

    fn status(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status")
            .exact_size(32.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(Margin::symmetric(18, 6)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                    ui.painter().circle_filled(
                        r.center(),
                        3.0,
                        if self.busy { AMBER } else { ACCENT },
                    );
                    ui.label(
                        RichText::new(if self.busy {
                            "Processing layers…"
                        } else {
                            "Ready"
                        })
                        .size(11.0)
                        .color(MUTED),
                    );
                    ui.add_space(18.0);
                    if let Some(p) = self.pointer {
                        ui.label(
                            RichText::new(format!("X {:>8.3}   Y {:>8.3} mm", p[0], p[1]))
                                .monospace()
                                .size(11.0),
                        );
                    } else {
                        ui.label(
                            RichText::new("All coordinates in millimeters")
                                .size(11.0)
                                .color(MUTED),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("{:.2} ms UI", self.cpu_time))
                                .size(11.0)
                                .color(MUTED),
                        )
                        .on_hover_text(
                            "CPU time building the previous UI frame; not GPU render time",
                        );
                        ui.separator();
                        ui.label(RichText::new("WGPU").size(10.0).strong().color(ACCENT))
                            .on_hover_text(&self.gpu);
                        ui.separator();
                        ui.label(
                            RichText::new(format!(
                                "{} primitives",
                                number(self.document.primitives())
                            ))
                            .size(11.0)
                            .color(MUTED),
                        );
                        if self.document.warning_count() > 0
                            && ui
                                .small_button(format!("{} warnings", self.document.warning_count()))
                                .clicked()
                        {
                            self.diagnostics = true;
                        }
                    });
                });
            });
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let collapsed = self.sidebar_collapsed;
        egui::Panel::left("layers")
            .exact_size(if collapsed { 48.0 } else { 292.0 })
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(Margin::symmetric(if collapsed { 7 } else { 18 }, 12)),
            )
            .show(ui, |ui| {
                if collapsed {
                    if icon_button(ui, Icon::ExpandSidebar, false)
                        .on_hover_text("Expand layer sidebar")
                        .clicked()
                    {
                        self.sidebar_collapsed = false;
                        ui.ctx().request_repaint();
                    }
                    ui.vertical_centered(|ui| {
                        icon(ui, Icon::Board, MUTED, 20.0);
                        ui.label(
                            RichText::new(self.document.layers.len().to_string())
                                .size(10.0)
                                .color(MUTED),
                        )
                        .on_hover_text(format!("{} layers", self.document.layers.len()));
                    });
                    return;
                }

                self.sidebar_board(ui);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("LAYERS ({})", self.document.layers.len()))
                            .size(10.0)
                            .strong()
                            .color(MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("Show all").clicked() {
                            self.solo_restore = None;
                            for layer in &mut self.document.layers {
                                layer.visible = true;
                            }
                        }
                    });
                });

                // Reserve the properties footer first; the list gets every
                // remaining point of height, with no outer/nested sidebar scroll.
                let warnings = !self.document.layers[self.selected].warnings.is_empty();
                let footer_height = (if warnings { 272.0_f32 } else { 240.0_f32 })
                    .min((ui.available_height() - 84.0).max(0.0));
                egui::Panel::bottom("layer-properties")
                    .exact_size(footer_height)
                    .resizable(false)
                    .show_separator_line(false)
                    .frame(egui::Frame::new())
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("layer-properties-scroll")
                            .auto_shrink([false, false])
                            .show(ui, |ui| self.sidebar_properties(ui));
                    });
                self.sidebar_layers(ui);
            });
    }

    fn sidebar_board(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("BOARD STACK")
                    .size(10.0)
                    .strong()
                    .color(MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if icon_button(ui, Icon::CollapseSidebar, false)
                    .on_hover_text("Collapse layer sidebar")
                    .clicked()
                {
                    self.sidebar_collapsed = true;
                    ui.ctx().request_repaint();
                }
            });
        });
        egui::Frame::new()
            .fill(CARD)
            .corner_radius(8)
            .inner_margin(12)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    icon(ui, Icon::Board, ACCENT, 24.0);
                    ui.vertical(|ui| {
                        let name = if self.document.demo {
                            "Atlas controller"
                        } else {
                            &self.document.name
                        };
                        ui.add(
                            egui::Label::new(RichText::new(name).size(15.0).strong()).truncate(),
                        )
                        .on_hover_text(name);
                        ui.label(
                            RichText::new(format!(
                                "{} layers · fabrication artwork",
                                self.document.layers.len()
                            ))
                            .size(11.0)
                            .color(MUTED),
                        );
                    });
                });
                ui.add_space(6.0);
                let size = self.document.bounds.size();
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{:.1} × {:.1}", size[0], size[1]))
                            .monospace()
                            .size(17.0),
                    );
                    ui.label(RichText::new("mm").size(11.0).color(MUTED));
                });
            });
    }

    fn sidebar_layers(&mut self, ui: &mut egui::Ui) {
        let mut clicked_solo = false;
        egui::ScrollArea::vertical()
            .id_salt("layer-scroll")
            .max_height(ui.available_height())
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for (i, layer) in self.document.layers.iter_mut().enumerate().rev() {
                    let (rect, _) = ui
                        .allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::hover());
                    let response = ui.interact(rect, ui.id().with(("layer", i)), Sense::click());
                    let selected = i == self.selected;
                    if selected && self.scroll_to_selected {
                        ui.scroll_to_rect(rect, Some(Align::Center));
                    }
                    ui.painter().rect_filled(
                        rect.shrink(1.0),
                        5,
                        if selected {
                            Color32::from_rgb(33, 47, 48)
                        } else if response.hovered() {
                            CARD
                        } else {
                            PANEL
                        },
                    );
                    if selected {
                        ui.painter().rect_stroke(
                            rect.shrink(1.0),
                            5,
                            Stroke::new(1.0, Color32::from_rgb(55, 83, 76)),
                            egui::StrokeKind::Inside,
                        );
                    }
                    let eye_rect = Rect::from_center_size(
                        Pos2::new(rect.left() + 19.0, rect.center().y),
                        Vec2::splat(28.0),
                    );
                    let eye = ui
                        .interact(eye_rect, ui.id().with(("eye", i)), Sense::click())
                        .on_hover_text(if layer.visible {
                            "Hide layer"
                        } else {
                            "Show layer"
                        });
                    draw_icon(
                        ui.painter(),
                        eye_rect.shrink(6.0),
                        Icon::Eye,
                        if layer.visible { TEXT } else { LINE },
                    );
                    if eye.clicked() {
                        layer.visible = !layer.visible;
                    }
                    let color = Color32::from_rgb(layer.color[0], layer.color[1], layer.color[2]);
                    let color = if layer.visible {
                        color
                    } else {
                        color.gamma_multiply(0.35)
                    };
                    ui.painter().rect_filled(
                        Rect::from_min_size(
                            Pos2::new(rect.left() + 39.0, rect.top() + 10.0),
                            Vec2::new(3.0, 20.0),
                        ),
                        1,
                        color,
                    );
                    let painter = ui.painter().with_clip_rect(Rect::from_min_max(
                        Pos2::new(rect.left() + 49.0, rect.top()),
                        Pos2::new(rect.right() - 8.0, rect.bottom()),
                    ));
                    painter.text(
                        Pos2::new(rect.left() + 51.0, rect.top() + 5.0),
                        Align2::LEFT_TOP,
                        layer.kind.label(),
                        FontId::proportional(12.0),
                        if layer.visible { TEXT } else { MUTED },
                    );
                    painter.text(
                        Pos2::new(rect.left() + 51.0, rect.top() + 22.0),
                        Align2::LEFT_TOP,
                        &layer.name,
                        FontId::proportional(10.0),
                        MUTED,
                    );
                    if response.clicked() && !eye.hovered() {
                        self.selected = i;
                    }
                    if response.double_clicked() && !eye.hovered() {
                        self.selected = i;
                        clicked_solo = true;
                    }
                    response.on_hover_text(format!(
                        "{}\n{} primitives · {} tools\nDouble-click to isolate",
                        layer.name,
                        number(layer.geometry.features.len()),
                        layer.apertures
                    ));
                }
            });
        self.scroll_to_selected = false;
        if clicked_solo {
            self.solo();
        }
    }

    fn sidebar_properties(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui
                .selectable_label(self.solo_restore.is_some(), "Isolate selected")
                .on_hover_text("Toggle isolation · S")
                .clicked()
            {
                self.solo();
            }
            if ui.small_button("Reset").clicked() {
                self.solo_restore = None;
                for layer in &mut self.document.layers {
                    layer.color = layer.kind.color();
                    layer.visible = layer.kind.default_visible();
                    layer.opacity = layer.kind.default_opacity();
                }
            }
        });
        ui.separator();
        ui.add_space(6.0);
        ui.label(
            RichText::new("LAYER PROPERTIES")
                .size(10.0)
                .strong()
                .color(MUTED),
        );
        ui.add_space(4.0);
        let layer = &mut self.document.layers[self.selected];
        ui.horizontal(|ui| {
            ui.color_edit_button_srgb(&mut layer.color);
            ui.label(RichText::new(layer.kind.label()).strong());
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Opacity").size(11.0).color(MUTED));
            ui.add(egui::Slider::new(&mut layer.opacity, 0.0..=1.0).show_value(false));
            ui.label(
                RichText::new(format!("{:.0}%", layer.opacity * 100.0))
                    .size(11.0)
                    .monospace(),
            );
        });
        ui.add_space(6.0);
        property(ui, "Primitives", &number(layer.geometry.features.len()));
        property(ui, "Apertures / tools", &layer.apertures.to_string());
        property(ui, "Triangles", &number(layer.geometry.indices.len() / 3));
        if !layer.warnings.is_empty()
            && ui
                .button(
                    RichText::new(format!("{} parser warnings", layer.warnings.len())).color(AMBER),
                )
                .clicked()
        {
            self.diagnostics = true;
        }
        ui.add_space(6.0);
        ui.separator();
        ui.label(
            RichText::new("Drop Gerbers, a folder, or a ZIP\nanywhere to open a board.")
                .size(11.0)
                .color(MUTED),
        );
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("tools")
            .exact_size(54.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(Margin::symmetric(18, 9)),
            )
            .show(ui, |ui| {
                let compact = ui.available_width() < 850.0;
                ui.horizontal_centered(|ui| {
                    for (tool, label, icon_kind, hint) in [
                        (Tool::Pan, "Pan", Icon::Hand, "H · Drag to pan"),
                        (
                            Tool::Inspect,
                            "Inspect",
                            Icon::Pointer,
                            "V · Select a feature by its bounds",
                        ),
                        (
                            Tool::Measure,
                            "Measure",
                            Icon::Ruler,
                            "M · Snap to centers, endpoints, and edges. Alt places a free point",
                        ),
                    ] {
                        let response = if compact {
                            icon_button(ui, icon_kind, self.tool == tool)
                        } else {
                            tool_button(ui, label, icon_kind, self.tool == tool)
                        };
                        if response.on_hover_text(hint).clicked() {
                            self.tool = tool;
                            self.measurement.clear();
                        }
                    }
                    if self.tool==Tool::Measure {
                        ui.add_space(8.0);
                        ui.toggle_value(&mut self.snap_enabled,"Snap")
                            .on_hover_text("N toggles snapping · Shift+N cycles modes · Hold Alt for a free point\nOnly visible, non-transparent layers participate");
                        if self.snap_enabled {
                            egui::ComboBox::from_id_salt("measure-snap-mode")
                                .selected_text(self.snap_mode.label()).width(80.0)
                                .show_ui(ui,|ui| {
                                    for mode in [SnapMode::Auto,SnapMode::Centers,SnapMode::Edges] {
                                        ui.selectable_value(&mut self.snap_mode,mode,mode.label());
                                    }
                                }).response.on_hover_text("Auto: centers first, then endpoints and edges\nCenters: declared pad/hole origins and trace/slot endpoints\nEdges: actual shape boundaries, not bounding boxes or mesh diagonals");
                        }
                    }
                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(12.0);
                    let fit = if compact {
                        icon_button(ui, Icon::Fit, false)
                    } else {
                        tool_button(ui, "Fit", Icon::Fit, false)
                    };
                    if fit
                        .on_hover_text("Fit board · F / Home / double-click")
                        .clicked()
                    {
                        self.fit_pending = true;
                    }
                    if icon_button(ui, Icon::Rotate, false)
                        .on_hover_text("Rotate 90° · R")
                        .clicked()
                    {
                        self.camera.rotation = (self.camera.rotation + 90.0) % 360.0;
                    }
                    if icon_button(ui, Icon::Mirror, self.camera.mirrored)
                        .on_hover_text("View from the back · B")
                        .clicked()
                    {
                        self.camera.mirrored = !self.camera.mirrored;
                    }
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        if icon_button(ui, Icon::Grid, self.grid)
                            .on_hover_text("Adaptive grid · G")
                            .clicked()
                        {
                            self.grid = !self.grid;
                        }
                        let substrate = if compact {
                            icon_button(ui, Icon::Board, self.substrate)
                        } else {
                            ui.selectable_label(self.substrate, "Substrate")
                        };
                        if substrate
                            .on_hover_text(
                                "Decorative board-bounds backdrop, not a manufactured outline",
                            )
                            .clicked()
                        {
                            self.substrate = !self.substrate;
                        }
                        if !compact {
                            ui.label(
                                RichText::new(if self.camera.mirrored {
                                    "BACK VIEW"
                                } else {
                                    "TOP VIEW"
                                })
                                .size(10.0)
                                .strong()
                                .color(MUTED),
                            );
                        }
                    });
                });
            });
    }

    fn canvas(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG)).show(ui,|ui| {
            let rect = ui.max_rect(); self.canvas = rect;
            let response = ui.interact(rect,ui.id().with("board-canvas"),Sense::click_and_drag());
            if self.fit_pending { self.camera.fit(self.document.visible_bounds(),rect); self.fit_pending = false; }
            self.pointer = response.hover_pos().map(|p| self.camera.world(p,rect));
            let space = ui.input(|i| i.key_down(Key::Space));
            if response.dragged_by(egui::PointerButton::Middle) || response.dragged_by(egui::PointerButton::Secondary)
                || (response.dragged_by(egui::PointerButton::Primary) && (self.tool == Tool::Pan || space)) {
                let delta = ui.input(|i| i.pointer.delta()); self.camera.pan(delta,rect);
            }
            if response.hovered() {
                let (scroll,pinch,ctrl) = ui.input(|i| (i.smooth_scroll_delta,i.zoom_delta(),i.modifiers.command));
                if pinch != 1.0 { self.camera.zoom(pinch,response.hover_pos().unwrap_or(rect.center()),rect); }
                else if scroll.y != 0.0 {
                    if ui.input(|i| i.modifiers.shift) { self.camera.pan(Vec2::new(scroll.y,scroll.x),rect); }
                    else { self.camera.zoom((scroll.y*if ctrl { 0.003 } else { 0.0025 }).exp(),response.hover_pos().unwrap_or(rect.center()),rect); }
                } else if scroll.x != 0.0 { self.camera.pan(Vec2::new(scroll.x,0.0),rect); }
                ui.ctx().set_cursor_icon(if self.tool == Tool::Pan || space { egui::CursorIcon::Grab } else { egui::CursorIcon::Crosshair });
            }
            if response.double_clicked() && self.tool == Tool::Pan { self.camera.fit(self.document.visible_bounds(),rect); }
            self.pointer = response.hover_pos().map(|p| self.camera.world(p,rect));
            self.snap_hover = if self.tool==Tool::Measure && self.snap_enabled && !space
                && !ui.input(|i|i.modifiers.alt)
                && !response.dragged_by(egui::PointerButton::Middle)
                && !response.dragged_by(egui::PointerButton::Secondary) {
                self.pointer.and_then(|p|self.document.snap(p,10.0/self.camera.scale,self.snap_mode,self.selected))
            } else { None };
            if let Some(snap)=self.snap_hover { self.pointer=Some(snap.hit.point); }
            if response.clicked() && !space && let Some(p) = self.pointer {
                if self.tool == Tool::Measure {
                    if self.measurement.len() == 2 { self.measurement.clear(); }
                    self.measurement.push(MeasurementPoint {point:p,snap:self.snap_hover});
                } else if self.tool == Tool::Inspect {
                    self.feature = self.document.layers.iter().enumerate().rev().filter(|(_,l)| l.visible)
                        .find_map(|(i,l)| l.geometry.pick(p,2.0/self.camera.scale).map(|j| (i,j)));
                    if let Some((layer,_)) = self.feature { self.selected = layer; }
                }
            }
            let painter = ui.painter().with_clip_rect(rect);
            if self.grid { self.paint_grid(&painter,rect); }
            if self.substrate {
                let a = self.camera.screen(self.document.bounds.min,rect); let b = self.camera.screen(self.document.bounds.max,rect);
                let board = Rect::from_two_pos(a,b);
                painter.rect_filled(board.translate(Vec2::new(3.0,5.0)).expand(3.0),8,Color32::from_black_alpha(45));
                painter.rect_filled(board,8,Color32::from_rgb(21,37,39));
                painter.rect_stroke(board,8,Stroke::new(1.0,Color32::from_rgb(36,56,57)),egui::StrokeKind::Inside);
            }
            painter.add(egui_wgpu::Callback::new_paint_callback(rect,BoardCallback {
                camera: self.camera,rect,generation: self.generation,
                layers: self.document.layers.iter().map(|l| RenderLayer { geometry: l.geometry.clone(),color: l.color,opacity: l.opacity,visible: l.visible }).collect(),
            }));
            self.paint_measurement(&painter,rect);
            self.paint_snap(&painter,rect);
            if let Some((i,j)) = self.feature {
                let f = &self.document.layers[i].geometry.features[j];
                let a = self.camera.screen(f.bounds.min,rect); let b = self.camera.screen(f.bounds.max,rect);
                painter.rect_stroke(Rect::from_two_pos(a,b).expand(4.0),3,Stroke::new(1.0,ACCENT),egui::StrokeKind::Outside);
                badge(&painter,Pos2::new(rect.left()+24.0,rect.bottom()-74.0),&format!("{} · bounds {:.3} × {:.3} mm",f.label,f.bounds.size()[0],f.bounds.size()[1]));
            }
            self.paint_scale(&painter,rect);
            painter.text(Pos2::new(rect.left()+24.0,rect.top()+22.0),Align2::LEFT_TOP,
                if self.busy { "LOADING BOARD…" } else { "FABRICATION WORKSPACE" },FontId::proportional(10.0),MUTED);
            if self.document.demo {
                painter.text(Pos2::new(rect.left()+24.0,rect.top()+42.0),Align2::LEFT_TOP,
                    "A little board. A lot of detail.",FontId::proportional(13.0),TEXT);
            }
            painter.text(Pos2::new(rect.center().x,rect.bottom()-24.0),Align2::CENTER_BOTTOM,
                match self.tool { Tool::Pan => "SCROLL TO ZOOM   ·   DRAG TO PAN   ·   F TO FIT",
                    Tool::Inspect => "CLICK TO INSPECT FEATURE BOUNDS   ·   ESC TO CLEAR",
                    Tool::Measure => "CLICK TWO POINTS   ·   N: SNAP ON/OFF   ·   ALT: FREE POINT   ·   SPACE + DRAG: PAN   ·   ESC: CLEAR" },
                FontId::proportional(9.0),MUTED);
            if ui.input(|i| !i.raw.hovered_files.is_empty()) {
                painter.rect_filled(rect,0,Color32::from_rgba_unmultiplied(13,28,26,235));
                painter.rect_stroke(rect.shrink(30.0),12,Stroke::new(2.0,ACCENT),egui::StrokeKind::Inside);
                painter.text(rect.center()-Vec2::new(0.0,20.0),Align2::CENTER_CENTER,"Drop your board here",FontId::proportional(28.0),ACCENT);
                painter.text(rect.center()+Vec2::new(0.0,24.0),Align2::CENTER_CENTER,"Gerber layers, Excellon drills, folders, or ZIP archives",FontId::proportional(14.0),TEXT);
            }
        });
    }

    fn paint_grid(&self, painter: &egui::Painter, rect: Rect) {
        let step = nice_step(24.0 / self.camera.scale);
        let view = self.camera.view_bounds(rect);
        for axis in 0..2 {
            let min = (view.min[axis] / step).floor() as i64;
            let max = (view.max[axis] / step).ceil() as i64;
            if max - min > 500 {
                continue;
            }
            for i in min..=max {
                let coordinate = i as f32 * step;
                let (a, b) = if axis == 0 {
                    ([coordinate, view.min[1]], [coordinate, view.max[1]])
                } else {
                    ([view.min[0], coordinate], [view.max[0], coordinate])
                };
                let color = if i == 0 {
                    Color32::from_rgb(40, 58, 62)
                } else if i % 5 == 0 {
                    Color32::from_rgb(28, 38, 47)
                } else {
                    Color32::from_rgb(20, 29, 37)
                };
                painter.line_segment(
                    [self.camera.screen(a, rect), self.camera.screen(b, rect)],
                    Stroke::new(1.0, color),
                );
            }
        }
        let origin = self.camera.screen([0.0, 0.0], rect);
        if rect.contains(origin) {
            painter.circle_stroke(origin, 4.0, Stroke::new(1.0, MUTED));
            painter.text(
                origin + Vec2::new(-8.0, 12.0),
                Align2::RIGHT_TOP,
                "0, 0",
                FontId::monospace(9.0),
                MUTED,
            );
        }
    }

    fn paint_measurement(&self, painter: &egui::Painter, rect: Rect) {
        if self.measurement.is_empty() {
            return;
        }
        let a = self.measurement[0].point;
        let b = self
            .measurement
            .get(1)
            .map(|p| p.point)
            .or(self.pointer)
            .unwrap_or(a);
        let sa = self.camera.screen(a, rect);
        let sb = self.camera.screen(b, rect);
        painter.line_segment([sa, sb], Stroke::new(1.5, ACCENT));
        for p in [sa, sb] {
            painter.circle_filled(p, 3.0, ACCENT);
            painter.circle_stroke(p, 7.0, Stroke::new(1.0, ACCENT));
        }
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let angle = dy.atan2(dx).to_degrees();
        let kind =
            |snap: Option<document::DocumentSnap>| snap.map_or("Free", |s| s.hit.kind.label());
        let from = kind(self.measurement[0].snap);
        let to = kind(self.measurement.get(1).map_or(self.snap_hover, |p| p.snap));
        badge(
            painter,
            Pos2::new(rect.left() + 24.0, rect.bottom() - 74.0),
            &format!(
                "{:.4} mm    ΔX {dx:.3}    ΔY {dy:.3}    {angle:.1}°    {from} → {to}",
                dx.hypot(dy)
            ),
        );
        badge(
            painter,
            sa.lerp(sb, 0.5) + Vec2::new(8.0, -28.0),
            &format!("{:.3} mm", dx.hypot(dy)),
        );
    }

    fn paint_snap(&self, painter: &egui::Painter, rect: Rect) {
        let Some(snap) = self.snap_hover else {
            return;
        };
        let p = self.camera.screen(snap.hit.point, rect);
        let stroke = Stroke::new(1.5, AMBER);
        match snap.hit.kind {
            SnapKind::Center => {
                painter.circle_stroke(p, 8.0, stroke);
                painter.line_segment([p - Vec2::new(12.0, 0.0), p + Vec2::new(12.0, 0.0)], stroke);
                painter.line_segment([p - Vec2::new(0.0, 12.0), p + Vec2::new(0.0, 12.0)], stroke);
            }
            SnapKind::Endpoint => {
                painter.rect_stroke(
                    Rect::from_center_size(p, Vec2::splat(12.0)),
                    1,
                    stroke,
                    egui::StrokeKind::Inside,
                );
            }
            SnapKind::Edge => {
                painter.circle_stroke(p, 5.0, stroke);
                let n = snap.hit.normal;
                let tangent = [
                    -n[1] * 10.0 / self.camera.scale,
                    n[0] * 10.0 / self.camera.scale,
                ];
                painter.line_segment(
                    [
                        self.camera.screen(
                            [
                                snap.hit.point[0] - tangent[0],
                                snap.hit.point[1] - tangent[1],
                            ],
                            rect,
                        ),
                        self.camera.screen(
                            [
                                snap.hit.point[0] + tangent[0],
                                snap.hit.point[1] + tangent[1],
                            ],
                            rect,
                        ),
                    ],
                    stroke,
                );
            }
        }
        let layer = &self.document.layers[snap.layer];
        let feature = &layer.geometry.features[snap.hit.feature];
        let text = format!(
            "{} · {} · {}",
            snap.hit.kind.label(),
            layer.kind.label(),
            feature.label
        );
        let size = painter
            .layout_no_wrap(text.clone(), FontId::monospace(11.0), ACCENT)
            .size()
            + Vec2::new(20.0, 16.0);
        let x = (p.x + 16.0)
            .min(rect.right() - size.x - 8.0)
            .max(rect.left() + 8.0);
        let y = (p.y + 16.0)
            .min(rect.bottom() - size.y - 30.0)
            .max(rect.top() + 8.0);
        badge(painter, Pos2::new(x, y), &text);
    }

    fn paint_scale(&self, painter: &egui::Painter, rect: Rect) {
        let length = nice_step(100.0 / self.camera.scale);
        let width = length * self.camera.scale;
        let p = Pos2::new(rect.right() - 28.0 - width, rect.bottom() - 45.0);
        painter.line_segment([p, p + Vec2::new(width, 0.0)], Stroke::new(1.0, MUTED));
        for x in [0.0, width] {
            painter.line_segment(
                [p + Vec2::new(x, -4.0), p + Vec2::new(x, 4.0)],
                Stroke::new(1.0, MUTED),
            );
        }
        painter.text(
            p + Vec2::new(width / 2.0, -10.0),
            Align2::CENTER_BOTTOM,
            format!("{length} mm"),
            FontId::monospace(10.0),
            MUTED,
        );
        let p = Pos2::new(rect.right() - 44.0, rect.top() + 75.0);
        let origin = self.camera.screen([0.0; 2], rect);
        let x = (self.camera.screen([1.0, 0.0], rect) - origin).normalized() * 23.0;
        let y = (self.camera.screen([0.0, 1.0], rect) - origin).normalized() * 23.0;
        for (v, color, label) in [(x, AMBER, "X"), (y, ACCENT, "Y")] {
            painter.arrow(p, v, Stroke::new(1.5, color));
            painter.text(
                p + v * 1.4,
                Align2::CENTER_CENTER,
                label,
                FontId::monospace(10.0),
                color,
            );
        }
    }

    fn windows(&mut self, ctx: &egui::Context) {
        egui::Window::new("Keyboard shortcuts")
            .open(&mut self.help)
            .resizable(false)
            .default_width(390.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("Stay in the flow.")
                        .size(22.0)
                        .strong()
                        .color(ACCENT),
                );
                ui.add_space(16.0);
                for (name, key) in [
                    ("Open files / folder", "Ctrl+O / Ctrl+Shift+O"),
                    ("Fit board", "F / Home"),
                    ("Pan / inspect / measure", "H / V / M"),
                    ("Measure snapping / free point", "N / hold Alt"),
                    ("Cycle snapping modes", "Shift+N"),
                    ("Temporary pan", "Space + drag / middle drag"),
                    ("Zoom at cursor", "Scroll / pinch"),
                    ("Rotate / flip board", "R / B"),
                    ("Isolate selected layer", "S / double-click layer"),
                    ("Toggle grid / layer", "G / 1–9"),
                    ("Clear measurement / selection", "Esc"),
                ] {
                    property(ui, name, key);
                }
                ui.add_space(12.0);
                ui.label(
                    RichText::new(
                        "Artwork viewer, not a CAM sign-off tool. Always review parser warnings.",
                    )
                    .color(MUTED)
                    .size(11.0),
                );
            });
        egui::Window::new("Import diagnostics")
            .open(&mut self.diagnostics)
            .default_width(600.0)
            .show(ctx, |ui| {
                ui.label(format!(
                    "Loaded {} layers in {:.1} ms",
                    self.document.layers.len(),
                    self.document.load_time.as_secs_f64() * 1000.0
                ));
                ui.label(format!(
                    "{} triangles · {:.1} MiB mesh",
                    number(self.document.triangles()),
                    self.document.memory_bytes() as f64 / 1048576.0
                ));
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(400.0)
                    .show(ui, |ui| {
                        for e in &self.document.errors {
                            ui.colored_label(AMBER, e);
                        }
                        for layer in &self.document.layers {
                            if !layer.warnings.is_empty() {
                                ui.label(RichText::new(&layer.name).strong());
                                for w in &layer.warnings {
                                    ui.colored_label(AMBER, w);
                                }
                            }
                        }
                        if self.document.warning_count() == 0 {
                            ui.colored_label(ACCENT, "No parser warnings.");
                        }
                    });
            });
        if let Some(error) = &self.error {
            let mut open = true;
            egui::Window::new("Workspace error")
                .open(&mut open)
                .default_width(540.0)
                .show(ctx, |ui| {
                    ui.colored_label(AMBER, error);
                    ui.add_space(12.0);
                    ui.label("Your current board has not been changed.");
                });
            if !open {
                self.error = None;
            }
        }
    }
}

impl eframe::App for App {
    #[cfg(target_arch = "wasm32")]
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let start = Instant::now();
        let ctx = ui.ctx().clone();
        while let Ok(message) = self.receiver.try_recv() {
            match message {
                Message::Loaded(result) => {
                    self.busy = false;
                    match result {
                        Ok(document) => self.accept(document),
                        Err(e) => {
                            if self.capture.is_some() {
                                eprintln!("Import failed: {e}");
                                std::process::exit(1);
                            }
                            self.error = Some(e);
                        }
                    }
                }
                #[cfg(not(target_arch = "wasm32"))]
                Message::Picked(paths) => self.load(paths, &ctx),
                #[cfg(not(target_arch = "wasm32"))]
                Message::Capture(path) => ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                    egui::UserData::new((path, false)),
                )),
                #[cfg(not(target_arch = "wasm32"))]
                Message::Error(error) => self.error = Some(error),
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let screenshots: Vec<_> = ctx.input(|i| {
                i.events
                    .iter()
                    .filter_map(|event| {
                        if let egui::Event::Screenshot {
                            image, user_data, ..
                        } = event
                        {
                            user_data
                                .data
                                .as_ref()?
                                .downcast_ref::<(PathBuf, bool)>()
                                .map(|(path, exit)| (image.clone(), path.clone(), *exit))
                        } else {
                            None
                        }
                    })
                    .collect()
            });
            for (image, path, exit) in screenshots {
                if exit {
                    if let Err(error) = save_png(&image, &path) {
                        eprintln!("Screenshot failed: {error}");
                        std::process::exit(1);
                    }
                    println!("Saved {}", path.display());
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                } else {
                    let sender = self.sender.clone();
                    let ctx = ctx.clone();
                    std::thread::spawn(move || {
                        if let Err(error) = save_png(&image, &path) {
                            let _ =
                                sender.send(Message::Error(format!("PNG export failed: {error}")));
                            ctx.request_repaint();
                        }
                    });
                }
            }
            let dropped: Vec<_> = ctx.input(|i| {
                i.raw
                    .dropped_files
                    .iter()
                    .map(|f| f.path().to_owned())
                    .collect()
            });
            if !dropped.is_empty() {
                self.load(dropped, &ctx);
            }
        }
        self.shortcuts(&ctx);
        self.header(ui);
        self.status(ui);
        self.sidebar(ui);
        self.toolbar(ui);
        self.canvas(ui);
        self.windows(&ctx);
        if self.capture.is_some() && !self.busy {
            self.capture_frames += 1;
            if self.capture_frames >= 4 {
                let path = self.capture.take().unwrap();
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new((
                    path, true,
                ))));
            }
            ctx.request_repaint();
        }
        if self.busy {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
        self.cpu_time = start.elapsed().as_secs_f32() * 1000.0;
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn save_png(image: &egui::ColorImage, path: &std::path::Path) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(file),
        image.size[0] as u32,
        image.size[1] as u32,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    let bytes: Vec<_> = image
        .pixels
        .iter()
        .flat_map(|p| p.to_srgba_unmultiplied())
        .collect();
    writer.write_image_data(&bytes).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())
}

fn theme(ctx: &egui::Context) {
    let mut style = egui::Style {
        visuals: egui::Visuals::dark(),
        ..Default::default()
    };
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.panel_fill = PANEL;
    style.visuals.window_fill = PANEL;
    style.visuals.extreme_bg_color = BG;
    style.visuals.faint_bg_color = CARD;
    style.visuals.selection.bg_fill = Color32::from_rgb(43, 78, 67);
    style.visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    style.visuals.widgets.inactive.bg_fill = CARD;
    style.visuals.widgets.inactive.weak_bg_fill = CARD;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, LINE);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(42, 56, 62);
    style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(42, 56, 62);
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, MUTED);
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(47, 78, 69);
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(10.0, 7.0);
    style.spacing.slider_width = 112.0;
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::proportional(13.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, FontId::proportional(12.0));
    style
        .text_styles
        .insert(egui::TextStyle::Small, FontId::proportional(10.0));
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_style_of(egui::Theme::Dark, style);
}

fn property(ui: &mut egui::Ui, name: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(name).size(11.0).color(MUTED));
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(value).size(11.0).monospace());
        });
    });
}

fn number(n: usize) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(c);
    }
    result
}

fn nice_step(value: f32) -> f32 {
    let base = 10.0_f32.powf(value.max(1e-9).log10().floor());
    let ratio = value / base;
    base * if ratio <= 1.0 {
        1.0
    } else if ratio <= 2.0 {
        2.0
    } else if ratio <= 5.0 {
        5.0
    } else {
        10.0
    }
}

fn pill(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.12))
        .corner_radius(4)
        .inner_margin(Margin::symmetric(7, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(color).strong().size(9.0));
        });
}

fn badge(painter: &egui::Painter, p: Pos2, text: &str) {
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::monospace(11.0), ACCENT);
    let r = Rect::from_min_size(p, galley.size() + Vec2::new(20.0, 16.0));
    painter.rect_filled(r, 5, PANEL);
    painter.rect_stroke(r, 5, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
    painter.galley(p + Vec2::new(10.0, 8.0), galley, ACCENT);
}

#[derive(Clone, Copy)]
enum Icon {
    Hand,
    Pointer,
    Ruler,
    Fit,
    Rotate,
    Mirror,
    Grid,
    Eye,
    Board,
    CollapseSidebar,
    ExpandSidebar,
}
fn tool_button(ui: &mut egui::Ui, label: &str, kind: Icon, selected: bool) -> egui::Response {
    let width = 34.0 + label.len() as f32 * 6.2;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 34.0), Sense::click());
    let color = if selected { ACCENT } else { MUTED };
    ui.painter().rect_filled(
        rect,
        5,
        if selected {
            Color32::from_rgb(37, 57, 51)
        } else if response.hovered() {
            CARD
        } else {
            PANEL
        },
    );
    draw_icon(
        ui.painter(),
        Rect::from_min_size(rect.min + Vec2::new(9.0, 9.0), Vec2::splat(16.0)),
        kind,
        color,
    );
    ui.painter().text(
        Pos2::new(rect.left() + 32.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(12.0),
        color,
    );
    response
}
fn icon_button(ui: &mut egui::Ui, kind: Icon, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::click());
    if selected || response.hovered() {
        ui.painter().rect_filled(rect, 5, CARD);
    }
    draw_icon(
        ui.painter(),
        rect.shrink(9.0),
        kind,
        if selected { ACCENT } else { MUTED },
    );
    response
}
fn icon(ui: &mut egui::Ui, kind: Icon, color: Color32, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    draw_icon(ui.painter(), rect, kind, color);
}
fn draw_icon(p: &egui::Painter, r: Rect, kind: Icon, color: Color32) {
    let at = |x: f32, y: f32| Pos2::new(r.left() + r.width() * x, r.top() + r.height() * y);
    let stroke = Stroke::new(1.3, color);
    let line = |a: (f32, f32), b: (f32, f32)| {
        p.line_segment([at(a.0, a.1), at(b.0, b.1)], stroke);
    };
    match kind {
        Icon::Hand => {
            for x in [0.35, 0.5, 0.65] {
                line((x, 0.52), (x, 0.15));
            }
            p.add(egui::Shape::line(
                vec![
                    at(0.2, 0.55),
                    at(0.2, 0.35),
                    at(0.3, 0.35),
                    at(0.3, 0.6),
                    at(0.7, 0.6),
                    at(0.75, 0.35),
                    at(0.85, 0.35),
                    at(0.8, 0.8),
                    at(0.65, 0.95),
                    at(0.35, 0.9),
                    at(0.2, 0.55),
                ],
                stroke,
            ));
        }
        Icon::Pointer => {
            p.add(egui::Shape::closed_line(
                vec![at(0.2, 0.05), at(0.85, 0.55), at(0.5, 0.6), at(0.35, 0.95)],
                stroke,
            ));
        }
        Icon::Ruler => {
            p.add(egui::Shape::closed_line(
                vec![at(0.0, 0.7), at(0.7, 0.0), at(1.0, 0.3), at(0.3, 1.0)],
                stroke,
            ));
            for i in 1..4 {
                let t = i as f32 * 0.18;
                line((t, 0.7 - t), (t + 0.15, 0.85 - t));
            }
        }
        Icon::Fit => {
            for (x, y, sx, sy) in [
                (0., 0., 1., 1.),
                (1., 0., -1., 1.),
                (0., 1., 1., -1.),
                (1., 1., -1., -1.),
            ] {
                line((x, y), (x + sx * 0.3, y));
                line((x, y), (x, y + sy * 0.3));
            }
        }
        Icon::Rotate => {
            p.add(egui::Shape::line(
                vec![
                    at(0.9, 0.4),
                    at(0.8, 0.2),
                    at(0.5, 0.1),
                    at(0.2, 0.25),
                    at(0.1, 0.6),
                    at(0.3, 0.85),
                    at(0.65, 0.9),
                ],
                stroke,
            ));
            line((0.9, 0.15), (0.9, 0.4));
            line((0.9, 0.4), (0.65, 0.4));
        }
        Icon::Mirror => {
            line((0.5, 0.0), (0.5, 1.0));
            p.add(egui::Shape::closed_line(
                vec![at(0.35, 0.15), at(0.0, 0.8), at(0.35, 0.8)],
                stroke,
            ));
            p.add(egui::Shape::closed_line(
                vec![at(0.65, 0.15), at(1.0, 0.8), at(0.65, 0.8)],
                stroke,
            ));
        }
        Icon::Grid => {
            for t in [0.1, 0.5, 0.9] {
                line((t, 0.0), (t, 1.0));
                line((0.0, t), (1.0, t));
            }
        }
        Icon::Eye => {
            p.add(egui::Shape::closed_line(
                vec![
                    at(0.0, 0.5),
                    at(0.25, 0.25),
                    at(0.5, 0.2),
                    at(0.75, 0.25),
                    at(1.0, 0.5),
                    at(0.75, 0.75),
                    at(0.5, 0.8),
                    at(0.25, 0.75),
                ],
                stroke,
            ));
            p.circle_filled(r.center(), r.width() * 0.12, color);
        }
        Icon::Board => {
            p.rect_stroke(r.shrink(3.0), 3, stroke, egui::StrokeKind::Inside);
            p.rect_stroke(r.shrink(8.0), 1, stroke, egui::StrokeKind::Inside);
            for x in [0.3, 0.5, 0.7] {
                line((x, 0.0), (x, 0.15));
                line((x, 0.85), (x, 1.0));
            }
        }
        Icon::CollapseSidebar | Icon::ExpandSidebar => {
            let (edge, tip) = if matches!(kind, Icon::CollapseSidebar) {
                (0.7, 0.3)
            } else {
                (0.3, 0.7)
            };
            line((edge, 0.15), (tip, 0.5));
            line((tip, 0.5), (edge, 0.85));
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (egui::Context, App) {
        let ctx = egui::Context::default();
        let mut app = App::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            vec![],
            None,
        );
        let source = "%FSLAX36Y36*%%MOMM*%%ADD10C,1*%D10*X0Y0D03*M02*";
        app.document = document::from_sources(
            "Sidebar test".into(),
            (0..43)
                .map(|i| (format!("test-layer-{i:02}.gtl"), source.into()))
                .collect(),
            false,
        )
        .unwrap();
        app.selected = 42;
        app.scroll_to_selected = false;
        (ctx, app)
    }

    fn frame(
        ctx: &egui::Context,
        app: &mut App,
        height: f32,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Rect) {
        let mut remaining = Rect::NOTHING;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, height))),
                events,
                ..Default::default()
            },
            |ui| {
                app.sidebar(ui);
                remaining = ui.available_rect_before_wrap();
            },
        );
        output.textures_delta.clear(); // Headless layout tests have no GPU uploader.
        (output, remaining)
    }

    fn text_positions(output: &egui::FullOutput, prefix: &str) -> Vec<Pos2> {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text)
                    if text.galley.job.text.starts_with(prefix)
                        && shape
                            .clip_rect
                            .intersects(text.galley.rect.translate(text.pos.to_vec2())) =>
                {
                    Some(text.pos)
                }
                _ => None,
            })
            .collect()
    }

    fn click(ctx: &egui::Context, app: &mut App, position: Pos2) {
        frame(ctx, app, 900.0, vec![egui::Event::PointerMoved(position)]);
        for pressed in [true, false] {
            frame(
                ctx,
                app,
                900.0,
                vec![egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        frame(ctx, app, 900.0, vec![]);
    }

    fn header_frame(
        ctx: &egui::Context,
        app: &mut App,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Rect) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 120.0))),
                events,
                ..Default::default()
            },
            |ui| app.header(ui),
        );
        output.textures_delta.clear();
        let icon = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect) if rect.fill_texture_id() == app.github_mark.id() => {
                    Some(rect.rect)
                }
                _ => None,
            })
            .expect("The header renders its bundled GitHub icon");
        (output, icon)
    }

    #[test]
    fn header_github_icon_sits_next_to_help_and_opens_the_repository_in_a_new_tab() {
        let (ctx, mut app) = fixture();
        header_frame(&ctx, &mut app, vec![]);
        let (output, icon) = header_frame(&ctx, &mut app, vec![]);
        let help = text_positions(&output, "?")[0];
        assert!(icon.right() < help.x && help.x - icon.right() < 40.0);
        assert_eq!(icon.size(), Vec2::splat(16.0));
        let position = icon.center();
        header_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(position)]);
        header_frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let (output, _) = header_frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(output.platform_output.commands.iter().any(|command| matches!(command,
            egui::OutputCommand::OpenUrl(url) if url.url == crate::brand::REPOSITORY_URL && url.new_tab)));
        assert!(
            !app.help,
            "The GitHub icon must not activate the adjacent help button"
        );
    }

    #[test]
    fn sidebar_counts_layers_and_grows_the_compact_list_with_window_height() {
        let (ctx, mut app) = fixture();
        frame(&ctx, &mut app, 900.0, vec![]);
        let (small, _) = frame(&ctx, &mut app, 900.0, vec![]);
        assert_eq!(text_positions(&small, "LAYERS (43)").len(), 1);
        let small_rows = text_positions(&small, "test-layer-");
        assert!(
            small_rows.len() >= 10,
            "Only {} rows visible",
            small_rows.len()
        );
        for pair in small_rows.windows(2) {
            assert!((pair[1].y - pair[0].y - 42.0).abs() < 0.1);
        }
        let footer =
            egui::containers::panel::PanelState::load(&ctx, egui::Id::new("layer-properties"))
                .unwrap()
                .outer_rect;
        assert!((footer.bottom() - 888.0).abs() < 0.1);
        let (large, _) = frame(&ctx, &mut app, 1300.0, vec![]);
        assert!(text_positions(&large, "test-layer-").len() >= small_rows.len() + 9);
        let larger_footer =
            egui::containers::panel::PanelState::load(&ctx, egui::Id::new("layer-properties"))
                .unwrap()
                .outer_rect;
        assert!((larger_footer.top() - footer.top() - 400.0).abs() < 0.1);
        assert!(!text_positions(&large, "Drop Gerbers").is_empty());
        for shape in &large.shapes {
            if let egui::Shape::Text(text) = &shape.shape
                && text.galley.job.text.starts_with("Drop Gerbers")
            {
                assert!(
                    shape
                        .clip_rect
                        .contains_rect(text.galley.rect.translate(text.pos.to_vec2())),
                    "The properties footer should not clip its final line at normal heights"
                );
            }
        }
    }

    #[test]
    fn sidebar_collapses_and_reopens_without_changing_board_or_selection() {
        let (ctx, mut app) = fixture();
        app.document.layers[3].visible = false;
        app.document.layers[42].opacity = 0.35;
        app.measurement.push(MeasurementPoint {
            point: [1.0, 2.0],
            snap: None,
        });
        let camera = app.camera;
        frame(&ctx, &mut app, 900.0, vec![]);
        let (_, expanded) = frame(&ctx, &mut app, 900.0, vec![]);
        assert_eq!(expanded.left(), 292.0);
        click(&ctx, &mut app, Pos2::new(257.0, 29.0));
        assert!(app.sidebar_collapsed);
        let (output, collapsed) = frame(&ctx, &mut app, 900.0, vec![]);
        assert_eq!(collapsed.left(), 48.0);
        assert!(text_positions(&output, "LAYER PROPERTIES").is_empty());
        click(&ctx, &mut app, Pos2::new(24.0, 29.0));
        assert!(!app.sidebar_collapsed);
        assert_eq!(frame(&ctx, &mut app, 900.0, vec![]).1.left(), 292.0);
        assert_eq!(app.selected, 42);
        assert!(!app.document.layers[3].visible);
        assert_eq!(app.document.layers[42].opacity, 0.35);
        assert_eq!(app.camera, camera);
        assert_eq!(app.measurement[0].point, [1.0, 2.0]);
    }

    #[test]
    fn compact_rows_keep_independent_visibility_and_selection_controls() {
        let (ctx, mut app) = fixture();
        frame(&ctx, &mut app, 900.0, vec![]);
        let (output, _) = frame(&ctx, &mut app, 900.0, vec![]);
        let row = text_positions(&output, "test-layer-")[1];
        click(&ctx, &mut app, Pos2::new(37.0, row.y - 2.0));
        assert!(!app.document.layers[41].visible);
        assert_eq!(app.selected, 42);
        click(&ctx, &mut app, Pos2::new(160.0, row.y - 2.0));
        assert_eq!(app.selected, 41);
        assert!(!app.document.layers[41].visible);
    }

    #[test]
    fn short_sidebar_reserves_list_space_and_keeps_warning_properties_scrollable() {
        let (ctx, mut app) = fixture();
        app.document.layers[42].warnings.push("Test warning".into());
        frame(&ctx, &mut app, 450.0, vec![]);
        let (output, _) = frame(&ctx, &mut app, 450.0, vec![]);
        let footer =
            egui::containers::panel::PanelState::load(&ctx, egui::Id::new("layer-properties"))
                .unwrap()
                .outer_rect;
        assert!(footer.height() < 272.0);
        assert!((footer.bottom() - 438.0).abs() < 0.1);
        assert!(!text_positions(&output, "test-layer-").is_empty());
        assert!(!text_positions(&output, "LAYER PROPERTIES").is_empty());
    }
}
