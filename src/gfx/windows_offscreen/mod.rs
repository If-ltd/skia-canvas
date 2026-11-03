use std::{
    cell::RefCell,
    sync::Mutex,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use skia_safe::{
    Color, ImageInfo, Surface,
    gpu::{self, Budgeted, DirectContext, SurfaceOrigin},
};

use super::{
    cache::Cache,
    engine::{RenderingEngine, render_thread},
    page::ExportOptions,
    vulkan::offscreen::VulkanContext,
};

mod d3d;
mod gl;

const CONTEXT_LIFESPAN: Duration = Duration::from_secs(5);

#[derive(Clone, Copy)]
enum Backend {
    D3d12,
    Vulkan,
    OpenGl,
}

impl Backend {
    fn name(self) -> &'static str {
        match self {
            Self::D3d12 => "D3D12",
            Self::Vulkan => "Vulkan",
            Self::OpenGl => "OpenGL",
        }
    }

    fn candidates(setting: &str) -> Result<&'static [Self], String> {
        match setting {
            "auto" => Ok(&[Self::Vulkan, Self::OpenGl]),
            "d3d12" => Ok(&[Self::D3d12]),
            "vulkan" => Ok(&[Self::Vulkan]),
            "opengl" => Ok(&[Self::OpenGl]),
            "cpu" => Ok(&[]),
            _ => Err(format!(
                "Invalid SKIA_CANVAS_BACKEND: {setting} (expected auto, d3d12, vulkan, opengl, or cpu)"
            )),
        }
    }

    fn open(self) -> Result<Context, String> {
        if std::env::var("MODA_SKIA_QA_SKIP")
            .is_ok_and(|list| list.split(',').any(|name| name == self.name()))
        {
            return Err("QA forced initialization failure".into());
        }
        let mut context = match self {
            Self::D3d12 => d3d::Context::new().map(Context::D3d12),
            Self::Vulkan => VulkanContext::new().map(Context::Vulkan),
            Self::OpenGl => gl::Context::new().map(Context::OpenGl),
        }?;
        // Verify the same draw and readback path used by bitmap exports.
        let info = ImageInfo::new_n32_premul((1, 1), None);
        let mut surface = context.surface(&info, &ExportOptions::default(), true)?;
        surface.canvas().clear(Color::WHITE);
        let mut pixel = [0; 4];
        if !surface.read_pixels(&info, &mut pixel, 4, (0, 0))
            || pixel != [255; 4]
            || context.is_lost()
        {
            return Err("offscreen draw/readback failed".into());
        }
        Ok(context)
    }
}

enum Context {
    D3d12(d3d::Context),
    Vulkan(VulkanContext),
    OpenGl(gl::Context),
}

impl Context {
    fn direct(&mut self) -> &mut DirectContext {
        match self {
            Self::D3d12(ctx) => &mut ctx.direct,
            Self::Vulkan(ctx) => &mut ctx.context,
            Self::OpenGl(ctx) => &mut ctx.direct,
        }
    }

    fn is_lost(&mut self) -> bool {
        matches!(self, Self::D3d12(ctx) if ctx.device_lost()) || self.direct().is_device_lost()
    }

    fn msaa(&mut self) -> Vec<usize> {
        let color_type = ImageInfo::new_n32_premul((1, 1), None).color_type();
        let max = self
            .direct()
            .max_surface_sample_count_for_color_type(color_type);
        [0, 2, 4, 8, 16].into_iter().filter(|&n| n <= max).collect()
    }

    fn surface(
        &mut self,
        info: &ImageInfo,
        opts: &ExportOptions,
        budgeted: bool,
    ) -> Result<Surface, String> {
        if let Self::Vulkan(ctx) = self {
            return ctx.surface(info, opts, budgeted);
        }

        let samples = opts.msaa_from(&self.msaa())?;
        gpu::surfaces::render_target(
            self.direct(),
            if budgeted {
                Budgeted::Yes
            } else {
                Budgeted::No
            },
            info,
            Some(samples),
            SurfaceOrigin::TopLeft,
            Some(&opts.surface_props()),
            false,
            None,
        )
        .ok_or_else(|| {
            format!(
                "Could not allocate new {}×{} bitmap",
                info.width(),
                info.height()
            )
        })
    }

    fn status(&mut self, backend: Backend) -> Value {
        let device = match self {
            Self::Vulkan(ctx) => return ctx.status(),
            Self::D3d12(ctx) => ctx.device.clone(),
            Self::OpenGl(ctx) => ctx.device.clone(),
        };
        json!({
            "renderer": "GPU",
            "api": backend.name(),
            "device": device,
            "msaa": self.msaa(),
            "threads": rayon::current_num_threads(),
        })
    }
}

struct State {
    candidates: &'static [Backend],
    failures: Vec<String>,
    context: Option<Context>,
    last_use: Instant,
}

impl State {
    fn new() -> Self {
        let setting = std::env::var("SKIA_CANVAS_BACKEND").unwrap_or_else(|_| "auto".into());
        let mut failures = Vec::new();
        let candidates = Backend::candidates(&setting).unwrap_or_else(|error| {
            failures.push(error);
            &[]
        });
        Self {
            candidates,
            failures,
            context: None,
            last_use: Instant::now(),
        }
    }

    fn ensure_context(&mut self) {
        if self.context.is_some() {
            return;
        }
        while let Some((&backend, rest)) = self.candidates.split_first() {
            match backend.open() {
                Ok(mut context) => {
                    self.publish_status(context.status(backend));
                    self.context = Some(context);
                    self.last_use = Instant::now();
                    return;
                }
                Err(error) => {
                    self.failures.push(format!("{}: {error}", backend.name()));
                    self.candidates = rest;
                }
            }
        }
        self.publish_status(cpu_status());
    }

    fn discard_lost_context(&mut self, cache: Cache) -> bool {
        if let Some(context) = self.context.as_mut()
            && context.is_lost()
        {
            context.direct().abandon();
            cache.clear(); // release textures and readback surfaces before dropping their context
            self.context = None;
            *STATUS.lock().unwrap() = None;
            let backend = self.candidates[0];
            self.failures
                .push(format!("{}: GPU device lost", backend.name()));
            self.candidates = &self.candidates[1..];
            return true;
        }
        false
    }

    fn publish_status(&self, mut status: Value) {
        if !self.failures.is_empty() {
            status["error"] = json!(self.failures.join("; "));
        }
        *STATUS.lock().unwrap() = Some(status);
    }
}

fn cpu_status() -> Value {
    json!({
        "renderer": "CPU",
        "api": null,
        "device": "CPU-based renderer",
        "driver": "N/A",
        "msaa": [0],
        "threads": rayon::current_num_threads(),
    })
}

// JS export validation reads engine status before submitting each job.
static STATUS: Mutex<Option<Value>> = Mutex::new(None);

fn cached_status() -> Option<Value> {
    STATUS.lock().unwrap().clone()
}

thread_local!(
    static STATE: RefCell<State> = RefCell::new(State::new());
);

pub(super) struct WindowsEngine;

impl WindowsEngine {
    pub(super) fn supported() -> bool {
        Self::status()["renderer"] == "GPU"
    }

    pub(super) fn status() -> Value {
        if let Some(status) = cached_status() {
            return status;
        }
        render_thread::run(|_| {
            if let Some(status) = cached_status() {
                return Ok(status);
            }
            STATE.with_borrow_mut(State::ensure_context);
            cached_status().ok_or("Windows backend status unavailable".into())
        })
        .unwrap_or_else(|error| {
            let mut status = cpu_status();
            status["error"] = json!(error);
            status
        })
    }

    pub(super) fn render<T>(
        cache: Cache,
        f: impl for<'a> Fn(Cache<'a>, RenderingEngine) -> Result<T, String>,
    ) -> Result<T, String> {
        loop {
            let gpu = STATE.with_borrow_mut(|state| {
                state.discard_lost_context(cache);
                state.ensure_context();
                state.last_use = Instant::now();
                state.context.is_some()
            });
            if !gpu {
                return f(Cache::shared(), RenderingEngine::CPU);
            }
            let result = f(cache, RenderingEngine::GPU);
            if !STATE.with_borrow_mut(|state| state.discard_lost_context(cache)) {
                return result;
            }
            // Only retry a lost device. Validation and unrelated rendering errors are returned.
        }
    }

    pub(super) fn make_surface(
        info: &ImageInfo,
        opts: &ExportOptions,
        budgeted: bool,
    ) -> Result<Surface, String> {
        STATE.with_borrow_mut(|state| {
            state
                .context
                .as_mut()
                .ok_or("GPU context unavailable".to_string())?
                .surface(info, opts, budgeted)
        })
    }

    pub(super) fn with_direct_context(f: impl FnOnce(Option<&mut DirectContext>)) {
        STATE.with_borrow_mut(|state| f(state.context.as_mut().map(Context::direct)));
    }

    pub(super) fn context_is_idle() -> bool {
        STATE.with_borrow(|state| {
            state.context.is_some() && state.last_use.elapsed() > CONTEXT_LIFESPAN
        })
    }

    pub(super) fn retire() {
        STATE.with_borrow_mut(|state| {
            state.context.take();
        });
    }

    pub(super) fn purge_stale() {
        STATE.with_borrow_mut(|state| {
            if let Some(context) = state.context.as_mut() {
                context
                    .direct()
                    .perform_deferred_cleanup(Duration::from_secs(1), None);
            }
        });
    }

    pub(super) fn with_cleanup<T>(f: impl FnOnce() -> T) -> T {
        f()
    }
}
