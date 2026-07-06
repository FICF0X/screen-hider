//! Screen Hider — GUI front-end.
//!
//! Lists open windows (like the "share screen" picker) and lets you toggle each
//! one's visibility in screen capture / screen share. Thin UI over `engine`.
#![windows_subsystem = "windows"] // no console window; errors surface in the status bar

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;
use std::time::Duration;

use eframe::egui;
use engine::{list_windows, set_affinity_local, set_hidden, WindowInfo};
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Language / i18n
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
enum Lang {
    Es,
    En,
}

impl Default for Lang {
    fn default() -> Self {
        Lang::Es // Spanish by default
    }
}

/// Translatable string keys.
enum T {
    Filter,
    Refresh,
    ShowAll,
    HideSelf,
    Options,
    Language,
    HideOnStart,
    Hotkey,
    Hide,
    Show,
    Unknown,
    Ready,
    HiddenFromCapture,
    Restored,
    Failed,
    SelfHidden,
    SelfVisible,
    OwnNotFound,
}

fn tr(lang: Lang, key: T) -> &'static str {
    use Lang::{En, Es};
    use T::*;
    match (lang, key) {
        (Es, Filter) => "Filtro:",
        (En, Filter) => "Filter:",
        (Es, Refresh) => "Actualizar",
        (En, Refresh) => "Refresh",
        (Es, ShowAll) => "Mostrar todo",
        (En, ShowAll) => "Show all",
        (Es, HideSelf) => "Ocultar Screen Hider de la captura",
        (En, HideSelf) => "Hide Screen Hider itself from capture",
        (Es, Options) => "Opciones",
        (En, Options) => "Options",
        (Es, Language) => "Idioma",
        (En, Language) => "Language",
        (Es, HideOnStart) => "Ocultar Screen Hider al iniciar",
        (En, HideOnStart) => "Hide Screen Hider on start",
        (Es, Hotkey) => "Atajo global: Ctrl+Alt+H (ocultar/mostrar todo)",
        (En, Hotkey) => "Global hotkey: Ctrl+Alt+H (hide/show all)",
        (Es, Hide) => "Ocultar",
        (En, Hide) => "Hide",
        (Es, Show) => "Mostrar",
        (En, Show) => "Show",
        (Es, Unknown) => "(desconocida)",
        (En, Unknown) => "(unknown)",
        (Es, Ready) => "Listo.",
        (En, Ready) => "Ready.",
        (Es, HiddenFromCapture) => "Oculta de la captura.",
        (En, HiddenFromCapture) => "Hidden from capture.",
        (Es, Restored) => "Restaurada.",
        (En, Restored) => "Restored.",
        (Es, Failed) => "Falló: la afinidad devolvió false.",
        (En, Failed) => "Failed: SetWindowDisplayAffinity returned false.",
        (Es, SelfHidden) => "Screen Hider oculto de la captura.",
        (En, SelfHidden) => "Screen Hider hidden from capture.",
        (Es, SelfVisible) => "Screen Hider visible de nuevo.",
        (En, SelfVisible) => "Screen Hider visible again.",
        (Es, OwnNotFound) => "No se encontró la propia ventana — probá Actualizar.",
        (En, OwnNotFound) => "Own window not found — press Refresh.",
    }
}

// ---------------------------------------------------------------------------
// Persisted settings
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Settings {
    lang: Lang,
    hide_self_on_start: bool,
    /// Window identities ("app|title") to auto-hide whenever they appear.
    remembered: Vec<String>,
}

fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    let mut p = PathBuf::from(base);
    p.push("ScreenHider");
    let _ = std::fs::create_dir_all(&p);
    p.push("config.json");
    Some(p)
}

fn load_settings() -> Settings {
    config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_settings(s: &Settings) {
    if let (Some(p), Ok(json)) = (config_path(), serde_json::to_string_pretty(s)) {
        let _ = std::fs::write(p, json);
    }
}

fn win_key(w: &WindowInfo) -> String {
    format!("{}|{}", w.app, w.title)
}

// ---------------------------------------------------------------------------
// Background worker
// ---------------------------------------------------------------------------

struct Job {
    pid: u32,
    hwnd: isize,
    hide: bool,
    key: String,
    /// Whether this toggle should be remembered across restarts. Manual per-window
    /// hides persist; the panic hotkey (hide-all) is transient.
    persist: bool,
}

struct JobResult {
    hwnd: isize,
    hide: bool,
    key: String,
    persist: bool,
    result: Result<bool, String>,
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

struct App {
    windows: Vec<WindowInfo>,
    hidden: HashSet<isize>,
    pending: HashSet<isize>,
    filter: String,
    status: String,
    own_pid: u32,
    own_hwnd: Option<isize>,
    self_hidden: bool,
    settings: Settings,
    show_options: bool,
    tx: Sender<Job>,
    rx: Receiver<JobResult>,
    _hotkey_mgr: Option<GlobalHotKeyManager>,
    hotkey_id: Option<u32>,
    hk_rx: Receiver<GlobalHotKeyEvent>,
}

impl App {
    fn new(ctx: egui::Context) -> Self {
        let (tx_job, rx_job) = std::sync::mpsc::channel::<Job>();
        let (tx_res, rx_res) = std::sync::mpsc::channel::<JobResult>();

        // Injection runs off the UI thread; catch_unwind keeps the worker alive
        // even if a single injection panics.
        let ctx_worker = ctx.clone();
        thread::spawn(move || {
            while let Ok(job) = rx_job.recv() {
                let Job {
                    pid, hwnd, hide, key, persist,
                } = job;
                let result = std::panic::catch_unwind(|| set_hidden(pid, hwnd, hide))
                    .unwrap_or_else(|_| {
                        Err("injection panicked (unsupported / protected process)".to_owned())
                    });
                let _ = tx_res.send(JobResult { hwnd, hide, key, persist, result });
                ctx_worker.request_repaint();
            }
        });

        // Register the global hotkey (Ctrl+Alt+H). Best-effort.
        let (mgr, hk_id) = match GlobalHotKeyManager::new() {
            Ok(m) => {
                let hk = HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyH);
                let id = hk.id();
                if m.register(hk).is_ok() {
                    (Some(m), Some(id))
                } else {
                    (Some(m), None)
                }
            }
            Err(_) => (None, None),
        };

        // Dedicated thread: block on hotkey events and wake the UI immediately,
        // so the hotkey works even while Screen Hider is in the background.
        let (hk_tx, hk_rx) = std::sync::mpsc::channel::<GlobalHotKeyEvent>();
        let ctx_hk = ctx.clone();
        thread::spawn(move || {
            let receiver = GlobalHotKeyEvent::receiver();
            while let Ok(ev) = receiver.recv() {
                if hk_tx.send(ev).is_err() {
                    break;
                }
                ctx_hk.request_repaint();
            }
        });

        let settings = load_settings();
        let lang = settings.lang;
        let mut app = Self {
            windows: Vec::new(),
            hidden: HashSet::new(),
            pending: HashSet::new(),
            filter: String::new(),
            status: tr(lang, T::Ready).to_owned(),
            own_pid: std::process::id(),
            own_hwnd: None,
            self_hidden: false,
            settings,
            show_options: false,
            tx: tx_job,
            rx: rx_res,
            _hotkey_mgr: mgr,
            hotkey_id: hk_id,
            hk_rx,
        };
        app.refresh();

        // Apply "hide self on start" preference.
        if app.settings.hide_self_on_start {
            if let Some(h) = app.own_hwnd {
                if set_affinity_local(h as i64, true) {
                    app.self_hidden = true;
                }
            }
        }
        app
    }

    fn lang(&self) -> Lang {
        self.settings.lang
    }

    fn refresh(&mut self) {
        let all = list_windows();
        self.own_hwnd = all.iter().find(|w| w.pid == self.own_pid).map(|w| w.hwnd);
        self.windows = all.into_iter().filter(|w| w.pid != self.own_pid).collect();
        self.status = format!("{} {}", self.windows.len(), if self.lang() == Lang::Es { "ventanas." } else { "windows." });

        // Auto-hide any window whose identity was remembered.
        let to_hide: Vec<(u32, isize, String)> = self
            .windows
            .iter()
            .filter(|w| {
                let k = win_key(w);
                self.settings.remembered.contains(&k)
                    && !self.hidden.contains(&w.hwnd)
                    && !self.pending.contains(&w.hwnd)
            })
            .map(|w| (w.pid, w.hwnd, win_key(w)))
            .collect();
        for (pid, hwnd, key) in to_hide {
            // Already remembered; re-applying should not re-persist.
            self.queue(pid, hwnd, true, key, false);
        }
    }

    fn queue(&mut self, pid: u32, hwnd: isize, hide: bool, key: String, persist: bool) {
        self.pending.insert(hwnd);
        let _ = self.tx.send(Job {
            pid,
            hwnd,
            hide,
            key,
            persist,
        });
    }

    fn show_all(&mut self) {
        let jobs: Vec<(u32, isize, String)> = self
            .windows
            .iter()
            .filter(|w| self.hidden.contains(&w.hwnd))
            .map(|w| (w.pid, w.hwnd, win_key(w)))
            .collect();
        for (pid, hwnd, key) in jobs {
            // Showing forgets the window (so it won't auto-hide next launch).
            self.queue(pid, hwnd, false, key, true);
        }
    }

    fn hide_all(&mut self) {
        let jobs: Vec<(u32, isize, String)> = self
            .windows
            .iter()
            .filter(|w| !self.hidden.contains(&w.hwnd) && !self.pending.contains(&w.hwnd))
            .map(|w| (w.pid, w.hwnd, win_key(w)))
            .collect();
        for (pid, hwnd, key) in jobs {
            // Panic hide-all is transient — do not remember these.
            self.queue(pid, hwnd, true, key, false);
        }
    }

    /// Ctrl+Alt+H: if anything is hidden, reveal all; otherwise hide everything.
    fn hotkey_toggle(&mut self) {
        if self.hidden.is_empty() {
            self.hide_all();
        } else {
            self.show_all();
        }
    }

    fn drain_results(&mut self) {
        let lang = self.lang();
        let mut changed = false;
        while let Ok(res) = self.rx.try_recv() {
            self.pending.remove(&res.hwnd);
            match res.result {
                Ok(true) => {
                    if res.hide {
                        self.hidden.insert(res.hwnd);
                        if res.persist && !self.settings.remembered.contains(&res.key) {
                            self.settings.remembered.push(res.key);
                            changed = true;
                        }
                        self.status = tr(lang, T::HiddenFromCapture).to_owned();
                    } else {
                        self.hidden.remove(&res.hwnd);
                        if res.persist {
                            if let Some(i) =
                                self.settings.remembered.iter().position(|k| *k == res.key)
                            {
                                self.settings.remembered.remove(i);
                                changed = true;
                            }
                        }
                        self.status = tr(lang, T::Restored).to_owned();
                    }
                }
                Ok(false) => self.status = tr(lang, T::Failed).to_owned(),
                Err(e) => self.status = format!("Error: {e}"),
            }
        }
        if changed {
            save_settings(&self.settings);
        }
    }

    fn poll_hotkey(&mut self) {
        while let Ok(ev) = self.hk_rx.try_recv() {
            if ev.state == HotKeyState::Pressed && Some(ev.id) == self.hotkey_id {
                self.hotkey_toggle();
            }
        }
    }

    fn toggle_self(&mut self) {
        let lang = self.lang();
        if let Some(h) = self.own_hwnd {
            let ok = set_affinity_local(h as i64, self.self_hidden);
            self.status = match (ok, self.self_hidden) {
                (true, true) => tr(lang, T::SelfHidden).to_owned(),
                (true, false) => tr(lang, T::SelfVisible).to_owned(),
                (false, _) => "SetWindowDisplayAffinity returned false.".to_owned(),
            };
        } else {
            self.self_hidden = false;
            self.status = tr(lang, T::OwnNotFound).to_owned();
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_hotkey();
        self.drain_results();
        let lang = self.lang();

        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.heading("Screen Hider");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(tr(lang, T::Refresh)).clicked() {
                        self.refresh();
                    }
                    if !self.hidden.is_empty() && ui.button(tr(lang, T::ShowAll)).clicked() {
                        self.show_all();
                    }
                    ui.toggle_value(&mut self.show_options, "⚙")
                        .on_hover_text(tr(lang, T::Options));
                });
            });

            // Collapsible options section.
            if self.show_options {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(tr(lang, T::Language));
                        let mut lang_now = self.settings.lang;
                        egui::ComboBox::from_id_salt("lang")
                            .selected_text(match lang_now {
                                Lang::Es => "Español",
                                Lang::En => "English",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut lang_now, Lang::Es, "Español");
                                ui.selectable_value(&mut lang_now, Lang::En, "English");
                            });
                        if lang_now != self.settings.lang {
                            self.settings.lang = lang_now;
                            save_settings(&self.settings);
                        }
                    });
                    if ui
                        .checkbox(&mut self.settings.hide_self_on_start, tr(lang, T::HideOnStart))
                        .changed()
                    {
                        save_settings(&self.settings);
                    }
                    ui.label(egui::RichText::new(tr(lang, T::Hotkey)).weak());
                });
            }

            ui.horizontal(|ui| {
                ui.label(tr(lang, T::Filter));
                ui.text_edit_singleline(&mut self.filter);
            });
            // Hide Screen Hider itself — no injection needed, it's our own window.
            if ui
                .checkbox(&mut self.self_hidden, tr(lang, T::HideSelf))
                .changed()
            {
                self.toggle_self();
            }
            ui.add_space(6.0);
        });

        egui::Panel::bottom("bottom").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(&self.status);
                if !self.hidden.is_empty() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let word = if lang == Lang::Es { "ocultas" } else { "hidden" };
                        ui.label(format!("{} {}", self.hidden.len(), word));
                    });
                }
            });
            ui.add_space(4.0);
        });

        egui::CentralPanel::default().show(ui, |ui| {
            let needle = self.filter.to_lowercase();
            let mut action: Option<(u32, isize, bool, String)> = None;

            egui::ScrollArea::vertical().show(ui, |ui| {
                let mut current_app: Option<String> = None;
                for w in &self.windows {
                    if !needle.is_empty()
                        && !w.title.to_lowercase().contains(&needle)
                        && !w.app.to_lowercase().contains(&needle)
                    {
                        continue;
                    }
                    if current_app.as_deref() != Some(w.app.as_str()) {
                        current_app = Some(w.app.clone());
                        ui.add_space(8.0);
                        let name = if w.app.is_empty() {
                            tr(lang, T::Unknown)
                        } else {
                            &w.app
                        };
                        ui.label(egui::RichText::new(name).strong().size(15.0));
                        ui.separator();
                    }
                    let is_hidden = self.hidden.contains(&w.hwnd);
                    let is_pending = self.pending.contains(&w.hwnd);
                    ui.horizontal(|ui| {
                        if is_pending {
                            ui.add_enabled(false, egui::Button::new("…"));
                        } else {
                            let btn = if is_hidden {
                                tr(lang, T::Show)
                            } else {
                                tr(lang, T::Hide)
                            };
                            if ui.button(btn).clicked() {
                                action = Some((w.pid, w.hwnd, !is_hidden, win_key(w)));
                            }
                        }
                        if is_hidden {
                            ui.colored_label(egui::Color32::from_rgb(220, 120, 60), "●");
                        } else {
                            ui.label("○");
                        }
                        ui.label(&w.title);
                    });
                }
            });

            if let Some((pid, hwnd, hide, key)) = action {
                // Manual per-window toggle — remember it across restarts.
                self.queue(pid, hwnd, hide, key, true);
            }
        });

        // Safety heartbeat. Hotkey presses and finished injections wake the UI
        // immediately via request_repaint(); this is just a periodic backstop.
        ui.ctx().request_repaint_after(Duration::from_secs(1));
    }
}

/// A simple generated icon: dark disc with an accent diagonal slash ("hidden").
fn app_icon() -> egui::IconData {
    let s: i32 = 64;
    let c = (s as f32 - 1.0) / 2.0;
    let mut rgba = vec![0u8; (s * s * 4) as usize];
    for y in 0..s {
        for x in 0..s {
            let idx = ((y * s + x) * 4) as usize;
            let (dx, dy) = (x as f32 - c, y as f32 - c);
            if (dx * dx + dy * dy).sqrt() > c {
                continue;
            }
            let (mut r, mut g, mut b) = (38u8, 42u8, 52u8);
            if ((x as f32 + y as f32) - (s as f32 - 1.0)).abs() < 6.0 {
                (r, g, b) = (224, 122, 63);
            }
            rgba[idx] = r;
            rgba[idx + 1] = g;
            rgba[idx + 2] = b;
            rgba[idx + 3] = 255;
        }
    }
    egui::IconData {
        rgba,
        width: s as u32,
        height: s as u32,
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([560.0, 680.0])
            .with_min_inner_size([420.0, 320.0])
            .with_icon(std::sync::Arc::new(app_icon())),
        ..Default::default()
    };
    eframe::run_native(
        "Screen Hider",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc.egui_ctx.clone())))),
    )
}
