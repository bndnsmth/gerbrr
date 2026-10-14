use crate::{app::App, document, transfer};
use js_sys::{Array, Object, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = gerbrrBridge,js_name = openFiles)]
    pub fn open_files(folder: bool);
    #[wasm_bindgen(js_namespace = gerbrrBridge,js_name = exportCanvas)]
    pub fn export_canvas();
}

#[wasm_bindgen]
pub struct BrowserApp {
    runner: eframe::WebRunner,
}

impl Default for BrowserApp {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl BrowserApp {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            runner: eframe::WebRunner::new(),
        }
    }

    pub async fn start(
        &self,
        canvas: web_sys::HtmlCanvasElement,
        force_webgl: bool,
    ) -> Result<(), JsValue> {
        use egui_wgpu::wgpu;

        // Presence of navigator.gpu alone is not enough: requestAdapter() can
        // return null. Pick WebGL2 before asking for a surface in that case.
        let backend = if force_webgl || !wgpu::util::is_browser_webgpu_supported().await {
            wgpu::Backends::GL
        } else {
            wgpu::Backends::BROWSER_WEBGPU
        };
        if backend == wgpu::Backends::GL {
            // wgpu reuses this context. Retain its drawing buffer so canvas PNG
            // export also works after idle frames, not just during a repaint.
            let attributes = Object::new();
            Reflect::set(&attributes, &"antialias".into(), &JsValue::FALSE)?;
            Reflect::set(&attributes, &"preserveDrawingBuffer".into(), &JsValue::TRUE)?;
            if canvas
                .get_context_with_context_options("webgl2", &attributes)?
                .is_none()
            {
                return Err(JsValue::from_str(
                    "Failed to create surface: browser could not provide a WebGL2 context",
                ));
            }
        }
        let mut options = eframe::WebOptions::default();
        if let egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup {
            setup.instance_descriptor.backends = backend;
        }
        self.runner
            .start(
                canvas,
                options,
                Box::new(|cc| Ok(Box::new(App::new(cc, vec![], None)))),
            )
            .await
    }

    pub fn begin_import(&self) -> bool {
        self.runner
            .app_mut::<App>()
            .is_some_and(|mut app| app.begin_browser_import())
    }

    pub fn finish_import(&self, bytes: &[u8]) -> Result<(), JsValue> {
        let result = transfer::decode(bytes);
        let error = result.as_ref().err().cloned();
        if let Some(mut app) = self.runner.app_mut::<App>() {
            app.complete_browser_import(result);
        }
        if let Some(error) = error {
            Err(JsValue::from_str(&error))
        } else {
            Ok(())
        }
    }

    pub fn fail_import(&self, error: String) {
        if let Some(mut app) = self.runner.app_mut::<App>() {
            app.complete_browser_import(Err(error));
        }
    }

    pub fn summary(&self) -> JsValue {
        let Some(app) = self.runner.app_mut::<App>() else {
            return JsValue::NULL;
        };
        let doc = app.browser_document();
        let object = Object::new();
        for (key, value) in [
            ("name", JsValue::from_str(&doc.name)),
            ("layers", JsValue::from_f64(doc.layers.len() as f64)),
            ("primitives", JsValue::from_f64(doc.primitives() as f64)),
            ("triangles", JsValue::from_f64(doc.triangles() as f64)),
            ("warnings", JsValue::from_f64(doc.warning_count() as f64)),
            ("gpu", JsValue::from_str(app.browser_gpu())),
            ("demo", JsValue::from_bool(doc.demo)),
            ("busy", JsValue::from_bool(app.browser_busy())),
            (
                "visibleLayers",
                JsValue::from_f64(doc.layers.iter().filter(|l| l.visible).count() as f64),
            ),
        ] {
            let _ = Reflect::set(&object, &JsValue::from_str(key), &value);
        }
        object.into()
    }

    /// Read-only measurement state for an HTML shell or browser smoke tests.
    pub fn measurement(&self) -> JsValue {
        let Some(app) = self.runner.app_mut::<App>() else {
            return JsValue::NULL;
        };
        let object = Object::new();
        let set = |object: &Object, key: &str, value: JsValue| {
            let _ = Reflect::set(object, &key.into(), &value);
        };
        let point = |p: &crate::app::MeasurementPoint| {
            let result = Object::new();
            set(&result, "x", JsValue::from_f64(p.point[0] as f64));
            set(&result, "y", JsValue::from_f64(p.point[1] as f64));
            set(
                &result,
                "kind",
                JsValue::from_str(p.snap.map_or("Free", |s| s.hit.kind.label())),
            );
            result
        };
        let points = Array::new();
        for p in app.browser_measurement() {
            points.push(&point(p));
        }
        set(&object, "points", points.into());
        set(
            &object,
            "preview",
            app.browser_measurement_preview()
                .map_or(JsValue::NULL, |p| point(&p).into()),
        );
        let (camera, canvas, enabled, mode) = app.browser_measurement_view();
        set(&object, "snapping", JsValue::from_bool(enabled));
        set(&object, "mode", mode.label().into());
        set(&object, "scale", JsValue::from_f64(camera.scale as f64));
        set(
            &object,
            "rotation",
            JsValue::from_f64(camera.rotation as f64),
        );
        set(&object, "mirrored", JsValue::from_bool(camera.mirrored));
        let center = Array::new();
        center.push(&JsValue::from_f64(camera.center[0] as f64));
        center.push(&JsValue::from_f64(camera.center[1] as f64));
        set(&object, "center", center.into());
        let rect = Array::new();
        for value in [canvas.left(), canvas.top(), canvas.width(), canvas.height()] {
            rect.push(&JsValue::from_f64(value as f64));
        }
        set(&object, "canvas", rect.into());
        let distance = app.browser_measurement().get(1).map(|b| {
            let a = app.browser_measurement()[0].point;
            (b.point[0] - a[0]).hypot(b.point[1] - a[1])
        });
        set(
            &object,
            "distance",
            distance.map_or(JsValue::NULL, |d| JsValue::from_f64(d as f64)),
        );
        object.into()
    }

    pub fn panic_message(&self) -> Option<String> {
        self.runner
            .panic_summary()
            .map(|summary| format!("{}\n{}", summary.message(), summary.callstack()))
    }
}

/// Called only inside the JS module worker. This does not create a window or GPU.
#[wasm_bindgen]
pub fn parse_files(names: Array, contents: Array) -> Result<Vec<u8>, JsValue> {
    if names.length() != contents.length() || names.length() == 0 || names.length() > 256 {
        return Err(JsValue::from_str("Select between 1 and 256 files"));
    }
    let mut files = Vec::new();
    for i in 0..names.length() {
        let name = names
            .get(i)
            .as_string()
            .ok_or_else(|| JsValue::from_str("Invalid file name"))?;
        let bytes = contents
            .get(i)
            .dyn_into::<Uint8Array>()
            .map_err(|_| JsValue::from_str("Invalid file bytes"))?;
        if bytes.length() > 128 * 1024 * 1024 {
            return Err(JsValue::from_str("File exceeds 128 MiB"));
        }
        files.push((name, bytes.to_vec()));
    }
    let doc = document::load_bytes(files).map_err(|e| JsValue::from_str(&e))?;
    transfer::encode(&doc).map_err(|e| JsValue::from_str(&e))
}
