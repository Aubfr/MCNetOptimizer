//! Etat global de l'application et boucle principale.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use mcnet_core::analysis::compare::TargetComparison;
use mcnet_core::analysis::diagnosis::Diagnosis;
use mcnet_core::config::Settings;
use mcnet_core::hotkey::{vk_for_function_key, HotkeyListener};
use mcnet_core::model::{SessionGroup, SessionMeta};
use mcnet_core::monitor::{LiveView, Monitor};
use mcnet_core::optimizer::{Optimizer, TweakStatus};
use mcnet_core::platform;
use mcnet_core::store::Store;
use mcnet_core::util::now_ms;

use crate::task::Task;
use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Dashboard,
    Diagnostic,
    Tweaks,
    History,
    Compare,
    Settings,
}

pub struct App {
    pub store: Arc<Store>,
    pub settings: Settings,
    pub optimizer: Arc<Optimizer>,
    pub page: Page,
    pub is_admin: bool,

    // Session
    pub monitor: Option<Monitor>,
    pub label: String,
    pub group: SessionGroup,
    pub auto_stop_min: u32,
    pub started_at: Option<Instant>,
    pub last_autosave: Instant,
    pub stop_task: Task<Result<String, String>>,
    pub view: Option<Arc<LiveView>>,
    pub view_at: Option<Instant>,

    // Marqueurs
    pub pending_markers: Arc<Mutex<Vec<u64>>>,
    pub hotkey: Option<HotkeyListener>,
    pub hotkey_error: Option<String>,

    // Diagnostic
    pub diag: Option<(String, Diagnosis)>,
    pub diag_task: Task<Result<(String, Diagnosis), String>>,
    pub diag_choice: Option<String>,

    // Optimisations
    pub tweaks: Vec<TweakStatus>,
    pub tweaks_loaded: bool,
    pub tweaks_task: Task<Vec<TweakStatus>>,
    pub tweak_action: Task<Result<String, String>>,

    // Historique
    pub history: Vec<SessionMeta>,
    pub selected: Option<String>,
    pub rename_buf: String,

    // Comparaison
    pub cmp: Option<(Vec<TargetComparison>, usize, usize)>,
    pub cmp_task: Task<Result<(Vec<TargetComparison>, usize, usize), String>>,

    pub toast: Option<(String, Instant, bool)>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> App {
        let store = Store::new(Store::default_dir())
            .or_else(|_| Store::new(std::env::temp_dir().join("McNetOptimizer")))
            .expect("impossible de créer le dossier de données");
        let settings = store.load_settings();
        let optimizer = Arc::new(Optimizer::with_default_tweaks(&store.dir));
        let history = store.list();
        let mut app = App {
            store: Arc::new(store),
            settings,
            optimizer,
            page: Page::Dashboard,
            is_admin: platform::is_admin(),
            monitor: None,
            label: String::new(),
            group: SessionGroup::None,
            auto_stop_min: 0,
            started_at: None,
            last_autosave: Instant::now(),
            stop_task: Task::new(),
            view: None,
            view_at: None,
            pending_markers: Arc::new(Mutex::new(Vec::new())),
            hotkey: None,
            hotkey_error: None,
            diag: None,
            diag_task: Task::new(),
            diag_choice: None,
            tweaks: Vec::new(),
            tweaks_loaded: false,
            tweaks_task: Task::new(),
            tweak_action: Task::new(),
            history,
            selected: None,
            rename_buf: String::new(),
            cmp: None,
            cmp_task: Task::new(),
            toast: None,
        };
        // Restauration automatique des reglages interrompus par un plantage.
        let recovered = app.optimizer.recover();
        if !recovered.is_empty() {
            app.toast(recovered.join(" "), true);
        }
        app.setup_hotkey(&cc.egui_ctx);
        app
    }

    pub fn toast(&mut self, msg: impl Into<String>, ok: bool) {
        self.toast = Some((msg.into(), Instant::now(), ok));
    }

    pub fn setup_hotkey(&mut self, ctx: &egui::Context) {
        if let Some(h) = self.hotkey.take() {
            h.stop();
        }
        self.hotkey_error = None;
        if !self.settings.hotkey_enabled {
            return;
        }
        let pending = self.pending_markers.clone();
        let ctx2 = ctx.clone();
        let cb: Box<dyn Fn(u64) + Send + 'static> = Box::new(move |t| {
            pending.lock().unwrap_or_else(|e| e.into_inner()).push(t);
            ctx2.request_repaint();
        });
        match HotkeyListener::start(
            self.settings.hotkey_modifiers(),
            vk_for_function_key(self.settings.hotkey_fn),
            self.settings.beep_on_marker,
            cb,
        ) {
            Ok(h) => self.hotkey = Some(h),
            Err(e) => self.hotkey_error = Some(e),
        }
    }

    pub fn save_settings(&mut self) {
        if let Err(e) = self.store.save_settings(&self.settings) {
            self.toast(format!("Enregistrement des paramètres impossible : {}", e), false);
        }
    }

    pub fn start_session(&mut self) {
        if self.monitor.is_some() {
            return;
        }
        let m = Monitor::start(
            &self.settings.monitor,
            self.label.trim().to_string(),
            self.group,
            self.optimizer.active_ids(),
        );
        self.monitor = Some(m);
        self.started_at = Some(Instant::now());
        self.last_autosave = Instant::now();
        self.view = None;
        self.view_at = None;
    }

    pub fn stop_session(&mut self, ctx: &egui::Context) {
        if let Some(m) = self.monitor.take() {
            let store = self.store.clone();
            self.stop_task.spawn(ctx, move || {
                let s = m.stop();
                store.save_session(&s).map(|_| s.id.clone())
            });
        }
        self.started_at = None;
        self.view = None;
    }

    pub fn add_marker_now(&mut self) {
        if let Some(m) = &self.monitor {
            m.add_marker(now_ms(), "");
        }
    }

    pub fn refresh_history(&mut self) {
        self.history = self.store.list();
    }

    pub fn refresh_tweaks(&mut self, ctx: &egui::Context) {
        let opt = self.optimizer.clone();
        self.tweaks_task.spawn(ctx, move || opt.status());
    }

    fn poll_tasks(&mut self, ctx: &egui::Context) {
        if let Some(r) = self.stop_task.take() {
            match r {
                Ok(id) => self.toast(format!("Session enregistrée : {}", id), true),
                Err(e) => self.toast(format!("Enregistrement impossible : {}", e), false),
            }
            self.refresh_history();
        }
        if let Some(r) = self.diag_task.take() {
            match r {
                Ok(d) => self.diag = Some(d),
                Err(e) => self.toast(e, false),
            }
        }
        if let Some(t) = self.tweaks_task.take() {
            self.tweaks = t;
            self.tweaks_loaded = true;
        }
        if let Some(r) = self.tweak_action.take() {
            match r {
                Ok(m) => self.toast(m, true),
                Err(e) => self.toast(e, false),
            }
            if let Some(m) = &self.monitor {
                m.set_active_tweaks(self.optimizer.active_ids());
            }
            self.refresh_tweaks(ctx);
        }
        if let Some(r) = self.cmp_task.take() {
            match r {
                Ok(c) => self.cmp = Some(c),
                Err(e) => self.toast(e, false),
            }
        }
    }

    fn nav_item(&mut self, ui: &mut egui::Ui, page: Page, label: &str) {
        let selected = self.page == page;
        let resp = ui.add_sized(
            [ui.available_width(), 36.0],
            egui::SelectableLabel::new(selected, egui::RichText::new(label).size(15.0)),
        );
        if resp.clicked() {
            self.page = page;
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_tasks(ctx);

        // Marqueurs poses par le raccourci global (horodatage exact de la pression).
        let pending: Vec<u64> = {
            let mut p = self.pending_markers.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *p)
        };
        if let Some(m) = &self.monitor {
            for t in pending {
                m.add_marker(t, "");
            }
        }

        // Instantane pour l'affichage (leger, rafraichi toutes les 400 ms).
        if self.monitor.is_some() {
            let stale = self.view_at.map(|t| t.elapsed() > Duration::from_millis(400)).unwrap_or(true);
            if stale {
                if let Some(m) = &self.monitor {
                    let v = m.view(self.settings.stats_window_s as u64, self.settings.chart_window_s as u64);
                    self.view = Some(Arc::new(v));
                    self.view_at = Some(Instant::now());
                }
            }
            // Arret automatique et sauvegarde periodique.
            let auto_done = match self.started_at {
                Some(t) => self.auto_stop_min > 0 && t.elapsed() >= Duration::from_secs(self.auto_stop_min as u64 * 60),
                None => false,
            };
            if auto_done {
                self.stop_session(ctx);
            } else if self.last_autosave.elapsed() > Duration::from_secs(120) {
                self.last_autosave = Instant::now();
                if let Some(m) = &self.monitor {
                    let s = m.snapshot_session();
                    let store = self.store.clone();
                    std::thread::spawn(move || {
                        let _ = store.save_session(&s);
                    });
                }
            }
        }

        egui::SidePanel::left("nav")
            .exact_width(220.0)
            .resizable(false)
            .frame(egui::Frame::none().fill(theme::SIDEBAR).inner_margin(egui::Margin::same(14.0)))
            .show(ctx, |ui| {
                ui.add_space(6.0);
                ui.label(egui::RichText::new("MC Net Optimizer").size(19.0).strong());
                ui.label(theme::muted("Minecraft 1.8.9 · Windows 11"));
                ui.add_space(14.0);
                self.nav_item(ui, Page::Dashboard, "Tableau de bord");
                self.nav_item(ui, Page::Diagnostic, "Diagnostic");
                self.nav_item(ui, Page::Tweaks, "Optimisations");
                self.nav_item(ui, Page::History, "Historique");
                self.nav_item(ui, Page::Compare, "Comparaison A/B");
                self.nav_item(ui, Page::Settings, "Paramètres");
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.add_space(4.0);
                    if self.is_admin {
                        theme::badge(ui, "Administrateur", theme::GOOD);
                    } else {
                        theme::badge(ui, "Sans droits admin", theme::WARN);
                    }
                    if self.monitor.is_some() {
                        theme::badge(ui, "Session en cours", theme::ACCENT);
                    } else {
                        theme::badge(ui, "Au repos", theme::MUTED);
                    }
                });
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(theme::BG).inner_margin(egui::Margin::same(18.0)))
            .show(ctx, |ui| {
                let expired = match &self.toast {
                    Some((_, t, _)) => t.elapsed() > Duration::from_secs(9),
                    None => false,
                };
                if expired {
                    self.toast = None;
                }
                if let Some((msg, _, ok)) = self.toast.clone() {
                    let color = if ok { theme::GOOD } else { theme::BAD };
                    egui::Frame::none()
                        .fill(color.gamma_multiply(0.15))
                        .stroke(egui::Stroke::new(1.0, color))
                        .rounding(8.0)
                        .inner_margin(egui::Margin::symmetric(12.0, 8.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(msg).color(color));
                                if ui.small_button("Fermer").clicked() {
                                    self.toast = None;
                                }
                            });
                        });
                    ui.add_space(8.0);
                }
                match self.page {
                    Page::Dashboard => self.ui_dashboard(ui),
                    Page::Diagnostic => self.ui_diagnostic(ui),
                    Page::Tweaks => self.ui_tweaks(ui),
                    Page::History => self.ui_history(ui),
                    Page::Compare => self.ui_compare(ui),
                    Page::Settings => self.ui_settings(ui),
                }
            });

        let running = self.monitor.is_some()
            || self.stop_task.running()
            || self.diag_task.running()
            || self.tweaks_task.running()
            || self.tweak_action.running();
        ctx.request_repaint_after(Duration::from_millis(if running { 500 } else { 1500 }));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(h) = self.hotkey.take() {
            h.stop();
        }
        if let Some(m) = self.monitor.take() {
            let s = m.stop();
            let _ = self.store.save_session(&s);
        }
        if self.settings.revert_on_exit {
            let _ = self.optimizer.revert_all();
        }
    }
}
