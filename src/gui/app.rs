//! The eframe application: picker (left), preview + ROI editor (center),
//! config + actions (right).

use crate::config::{Config, RoiHint, RoiKind, Target};
use crate::error::Error;
use crate::frame::{RawFrame, WindowInfo};
use crate::{ControlFlow, DirectorySink};
use egui::{Color32, ColorImage, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const PREVIEW_MAX_W: u32 = 720;

/// Shared slot for the most recent preview frame from the capture thread.
struct Shared {
    latest: Mutex<Option<RawFrame>>,
    stop: AtomicBool,
}

struct DragState {
    start: Pos2,
    current: Pos2,
}

struct FrameWatchApp {
    windows: Vec<WindowInfo>,
    selected: Option<usize>,
    config: Config,
    status: String,

    shared: Arc<Shared>,
    capture_running: bool,
    /// Join handle for the current preview worker, so the prior one is stopped
    /// and joined before a new selection starts (no leaked capture threads).
    preview_handle: Option<std::thread::JoinHandle<()>>,
    /// True while a `watch` session thread is running, so repeated "Start
    /// watching" clicks can't spawn unbounded concurrent sessions.
    watch_active: Arc<AtomicBool>,
    texture: Option<TextureHandle>,

    new_kind: RoiKind,
    new_label: String,
    drag: Option<DragState>,
}

impl FrameWatchApp {
    fn new(initial: Option<Config>) -> Self {
        let mut config = initial.unwrap_or_default();
        if config.out_dir.as_os_str().is_empty() {
            config.out_dir = "./.framewatch".into();
        }
        let mut app = Self {
            windows: Vec::new(),
            selected: None,
            config,
            status: "Select a window to begin.".into(),
            shared: Arc::new(Shared {
                latest: Mutex::new(None),
                stop: AtomicBool::new(false),
            }),
            capture_running: false,
            preview_handle: None,
            watch_active: Arc::new(AtomicBool::new(false)),
            texture: None,
            new_kind: RoiKind::Spinner,
            new_label: String::new(),
            drag: None,
        };
        app.refresh_windows();
        app
    }

    fn refresh_windows(&mut self) {
        match crate::enumerate_windows() {
            Ok(list) => {
                self.windows = list;
                self.status = format!("{} capturable windows.", self.windows.len());
            }
            Err(e) => {
                self.windows.clear();
                self.status = format!("Enumeration unavailable: {e}");
            }
        }
    }

    fn start_preview(&mut self, hwnd: isize) {
        // Stop and join the prior worker first, then give the new one its *own*
        // Shared so the two can never race on `latest`/`stop`.
        self.stop_preview();
        let mut cfg = self.config.clone();
        cfg.target = Target::ByHwnd(hwnd);
        let shared = Arc::new(Shared {
            latest: Mutex::new(None),
            stop: AtomicBool::new(false),
        });
        self.shared = shared.clone();

        let handle = std::thread::spawn(move || {
            let mut backend = match crate::default_backend(&cfg) {
                Ok(b) => b,
                Err(_) => return,
            };
            let _ = backend.run(&mut |frame| {
                if let Ok(mut slot) = shared.latest.lock() {
                    *slot = Some(frame);
                }
                if shared.stop.load(Ordering::Relaxed) {
                    ControlFlow::Stop
                } else {
                    ControlFlow::Continue
                }
            });
        });
        self.preview_handle = Some(handle);
        self.capture_running = true;
    }

    fn stop_preview(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.preview_handle.take() {
            let _ = handle.join();
        }
        self.capture_running = false;
    }

    /// Pull the latest frame (if any) into a (downscaled) egui texture.
    fn update_texture(&mut self, ctx: &egui::Context) {
        let frame = self.shared.latest.lock().ok().and_then(|g| g.clone());
        if let Some(frame) = frame {
            let img = downscale_to_color_image(&frame, PREVIEW_MAX_W);
            self.texture = Some(ctx.load_texture("preview", img, TextureOptions::LINEAR));
        }
    }

    fn start_watching(&mut self) {
        let Some(idx) = self.selected else {
            self.status = "Select a window first.".into();
            return;
        };
        if self.watch_active.load(Ordering::Relaxed) {
            self.status = "A watch session is already running.".into();
            return;
        }
        let hwnd = self.windows[idx].hwnd;
        let mut cfg = self.config.clone();
        cfg.target = Target::ByHwnd(hwnd);
        match DirectorySink::new(&cfg) {
            Ok(sink) => {
                let dir = sink.session().dir.clone();
                self.status = format!("Watching → {}", dir.display());
                // Bound to one concurrent session: the flag is cleared when the
                // session thread exits (window closed / stop / error).
                self.watch_active.store(true, Ordering::Relaxed);
                let active = self.watch_active.clone();
                std::thread::spawn(move || {
                    let _ = crate::watch(cfg, sink);
                    active.store(false, Ordering::Relaxed);
                });
            }
            Err(e) => self.status = format!("Failed to start: {e}"),
        }
    }

    fn save_config(&mut self) {
        match self.config.to_toml_string() {
            Ok(toml) => {
                let path = std::path::Path::new("framewatch.toml");
                match std::fs::write(path, toml) {
                    Ok(()) => self.status = "Saved framewatch.toml".into(),
                    Err(e) => self.status = format!("Save failed: {e}"),
                }
                self.save_rois_per_user();
            }
            Err(e) => self.status = format!("Serialize failed: {e}"),
        }
    }

    fn save_rois_per_user(&self) {
        let Some(idx) = self.selected else { return };
        let w = &self.windows[idx];
        let key = sanitize(&format!("{}_{}", w.class, w.exe));
        if let Some(base) = dirs::config_dir() {
            let dir = base.join("framewatch").join("rois");
            if std::fs::create_dir_all(&dir).is_ok() {
                if let Ok(json) = serde_json::to_string_pretty(&self.config.rois) {
                    let _ = std::fs::write(dir.join(format!("{key}.json")), json);
                }
            }
        }
    }

    fn draw_preview(&mut self, ui: &mut egui::Ui) {
        let Some(tex) = self.texture.clone() else {
            ui.centered_and_justified(|ui| {
                ui.label("No preview. Select a window (live capture needs the `wgc` feature).");
            });
            return;
        };

        let tex_size = tex.size_vec2();
        let avail = ui.available_size();
        let scale = (avail.x / tex_size.x).min(avail.y / tex_size.y).min(1.0);
        let draw_size = tex_size * scale;
        let (rect, response) = ui.allocate_exact_size(draw_size, Sense::click_and_drag());

        let painter = ui.painter_at(rect);
        painter.image(
            tex.id(),
            rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );

        // Existing ROIs.
        for roi in &self.config.rois {
            let r = norm_to_screen(roi.rect_norm, rect);
            let color = kind_color(roi.kind);
            painter.rect_stroke(r, 0.0, Stroke::new(2.0, color), egui::StrokeKind::Middle);
            painter.text(
                r.left_top(),
                egui::Align2::LEFT_BOTTOM,
                &roi.label,
                egui::FontId::proportional(12.0),
                color,
            );
        }

        // New ROI drag.
        if response.drag_started() {
            if let Some(p) = response.interact_pointer_pos() {
                self.drag = Some(DragState {
                    start: p,
                    current: p,
                });
            }
        }
        if let Some(d) = self.drag.as_mut() {
            if let Some(p) = response.interact_pointer_pos() {
                d.current = p;
            }
            let dragging = Rect::from_two_pos(d.start, d.current);
            painter.rect_stroke(
                dragging,
                0.0,
                Stroke::new(2.0, kind_color(self.new_kind)),
                egui::StrokeKind::Middle,
            );
        }
        if response.drag_stopped() {
            if let Some(d) = self.drag.take() {
                let dragging = Rect::from_two_pos(d.start, d.current).intersect(rect);
                if dragging.width() > 3.0 && dragging.height() > 3.0 {
                    let rn = screen_to_norm(dragging, rect);
                    let label = if self.new_label.is_empty() {
                        format!("{:?}-{}", self.new_kind, self.config.rois.len())
                    } else {
                        self.new_label.clone()
                    };
                    self.config.rois.push(RoiHint {
                        kind: self.new_kind,
                        label,
                        rect_norm: rn,
                    });
                }
            }
        }
    }
}

impl eframe::App for FrameWatchApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_texture(ctx);

        egui::SidePanel::left("picker")
            .exact_width(260.0)
            .show(ctx, |ui| {
                ui.heading("Windows");
                if ui.button("⟳ Refresh").clicked() {
                    self.refresh_windows();
                }
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut to_select = None;
                    for (i, w) in self.windows.iter().enumerate() {
                        let label = format!("{} — {}", truncate(&w.title, 40), w.exe);
                        if ui
                            .selectable_label(self.selected == Some(i), label)
                            .clicked()
                        {
                            to_select = Some((i, w.hwnd));
                        }
                    }
                    if let Some((i, hwnd)) = to_select {
                        self.selected = Some(i);
                        self.start_preview(hwnd);
                    }
                });
            });

        egui::SidePanel::right("config")
            .exact_width(260.0)
            .show(ctx, |ui| {
                ui.heading("Config");
                ui.add(egui::Slider::new(&mut self.config.settle_ms, 50..=2000).text("settle ms"));
                ui.add(
                    egui::Slider::new(&mut self.config.value_sample_ms, 100..=5000)
                        .text("value sample ms"),
                );
                ui.add(
                    egui::Slider::new(&mut self.config.tile_change_threshold, 1..=64)
                        .text("tile sensitivity"),
                );
                ui.separator();

                ui.label("New region kind:");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.new_kind, RoiKind::Watch, "Watch");
                    ui.selectable_value(&mut self.new_kind, RoiKind::Spinner, "Spinner");
                });
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.new_kind, RoiKind::Volatile, "Volatile");
                    ui.selectable_value(&mut self.new_kind, RoiKind::Ignore, "Ignore");
                });
                ui.horizontal(|ui| {
                    ui.label("label:");
                    ui.text_edit_singleline(&mut self.new_label);
                });
                ui.label("Drag on the preview to draw a region.");
                ui.separator();

                ui.label("Regions:");
                let mut remove = None;
                for (i, roi) in self.config.rois.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.colored_label(kind_color(roi.kind), format!("{:?}", roi.kind));
                        ui.label(&roi.label);
                        if ui.small_button("✕").clicked() {
                            remove = Some(i);
                        }
                    });
                }
                if let Some(i) = remove {
                    self.config.rois.remove(i);
                }
                ui.separator();

                if ui.button("💾 Save config & ROIs").clicked() {
                    self.save_config();
                }
                if ui.button("▶ Start watching").clicked() {
                    self.start_watching();
                }
            });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.label(&self.status);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            self.draw_preview(ui);
        });

        // Keep the preview live.
        if self.capture_running {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

/// Launch the GUI.
pub fn run(initial: Option<Config>) -> Result<(), Error> {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "framewatch",
        options,
        Box::new(move |_cc| Ok(Box::new(FrameWatchApp::new(initial)))),
    )
    .map_err(|e| Error::Config(format!("gui error: {e}")))
}

fn kind_color(kind: RoiKind) -> Color32 {
    match kind {
        RoiKind::Watch => Color32::from_rgb(80, 200, 120),
        RoiKind::Spinner => Color32::from_rgb(240, 180, 40),
        RoiKind::Volatile => Color32::from_rgb(90, 160, 240),
        RoiKind::Ignore => Color32::from_rgb(220, 80, 80),
    }
}

fn norm_to_screen(rn: [f32; 4], rect: Rect) -> Rect {
    Rect::from_min_size(
        Pos2::new(
            rect.min.x + rn[0] * rect.width(),
            rect.min.y + rn[1] * rect.height(),
        ),
        egui::vec2(rn[2] * rect.width(), rn[3] * rect.height()),
    )
}

fn screen_to_norm(r: Rect, base: Rect) -> [f32; 4] {
    [
        (r.min.x - base.min.x) / base.width(),
        (r.min.y - base.min.y) / base.height(),
        r.width() / base.width(),
        r.height() / base.height(),
    ]
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect::<String>() + "…"
    }
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Downscale a BGRA frame to an egui `ColorImage` (RGBA) at most `max_w` wide.
fn downscale_to_color_image(frame: &RawFrame, max_w: u32) -> ColorImage {
    let scale = if frame.width > max_w {
        frame.width as f32 / max_w as f32
    } else {
        1.0
    };
    let out_w = (frame.width as f32 / scale).round().max(1.0) as u32;
    let out_h = (frame.height as f32 / scale).round().max(1.0) as u32;
    let mut pixels = Vec::with_capacity((out_w * out_h) as usize);
    for y in 0..out_h {
        let sy = (y as f32 * scale) as u32;
        let row = (sy * frame.stride) as usize;
        for x in 0..out_w {
            let sx = (x as f32 * scale) as u32;
            let off = row + (sx * 4) as usize;
            let b = frame.buffer.get(off).copied().unwrap_or(0);
            let g = frame.buffer.get(off + 1).copied().unwrap_or(0);
            let r = frame.buffer.get(off + 2).copied().unwrap_or(0);
            let a = frame.buffer.get(off + 3).copied().unwrap_or(255);
            pixels.push(Color32::from_rgba_unmultiplied(r, g, b, a));
        }
    }
    ColorImage::new([out_w as usize, out_h as usize], pixels)
}
