//! Tableau de bord temps reel.

use eframe::egui::{self, Color32, RichText};
use mcnet_core::model::{SessionGroup, Target};
use mcnet_core::monitor::TargetView;
use mcnet_core::util::{fmt_duration, fmt_time};

use crate::app::App;
use crate::chart::{latency_chart, ChartSeries};
use crate::theme;

fn series_color(t: Target) -> Color32 {
    match t {
        Target::Gateway => theme::GOOD,
        Target::Internet => theme::ACCENT,
        Target::Server => Color32::from_rgb(163, 113, 247),
        Target::Game => Color32::from_rgb(57, 197, 187),
    }
}

fn fmt_last(last: Option<Option<f32>>) -> (String, Color32) {
    match last {
        Some(Some(v)) => (format!("{:.1} ms", v), theme::TEXT),
        Some(None) => ("perte".to_string(), theme::BAD),
        None => ("—".to_string(), theme::MUTED),
    }
}

fn target_card(ui: &mut egui::Ui, tv: &TargetView, reason: Option<String>) {
    theme::card(ui, |ui| {
        ui.set_min_height(128.0);
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().circle_filled(r.center(), 5.0, series_color(tv.target));
            ui.label(RichText::new(tv.target.label()).strong());
        });
        let (txt, col) = fmt_last(tv.last);
        ui.label(RichText::new(txt).size(26.0).strong().color(col));
        if tv.stats.has_data() {
            let c = theme::quality_color(tv.stats.jitter, tv.stats.loss_pct);
            egui::Grid::new(format!("tc-{}", tv.target.short())).num_columns(2).spacing([14.0, 2.0]).show(ui, |ui| {
                ui.label(theme::muted("médiane"));
                ui.label(format!("{:.1} ms", tv.stats.p50));
                ui.end_row();
                ui.label(theme::muted("p95"));
                ui.label(format!("{:.1} ms", tv.stats.p95));
                ui.end_row();
                ui.label(theme::muted("gigue"));
                ui.label(RichText::new(format!("{:.1} ms", tv.stats.jitter)).color(c));
                ui.end_row();
                ui.label(theme::muted("perte"));
                ui.label(RichText::new(format!("{:.2} %", tv.stats.loss_pct)).color(c));
                ui.end_row();
            });
        } else if let Some(r) = reason {
            ui.label(theme::muted(&r));
        } else {
            ui.label(theme::muted("en attente de mesures…"));
        }
    });
}

impl App {
    pub fn ui_dashboard(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            ui.heading("Tableau de bord");
        });
        ui.label(theme::muted(
            "Mesure en continu la box, Internet, le serveur et la vraie connexion de jeu. Observation seule : aucun paquet n'est intercepté ni modifié.",
        ));
        ui.add_space(8.0);

        // ---- Controles de session ----
        let running = self.monitor.is_some();
        let stopping = self.stop_task.running();
        let marker_count = self.monitor.as_ref().map(|m| m.marker_count()).unwrap_or(0);
        let mut start = false;
        let mut stop = false;
        let mut mark = false;
        theme::card(ui, |ui| {
            if stopping {
                ui.label("Arrêt des sondes et enregistrement de la session…");
                return;
            }
            if !running {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Libellé :");
                    ui.add(egui::TextEdit::singleline(&mut self.label).desired_width(190.0).hint_text("ex. sans réglage"));
                    ui.label("Groupe A/B :");
                    egui::ComboBox::from_id_source("grp")
                        .selected_text(match self.group {
                            SessionGroup::None => "Aucun",
                            SessionGroup::A => "A",
                            SessionGroup::B => "B",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.group, SessionGroup::None, "Aucun");
                            ui.selectable_value(&mut self.group, SessionGroup::A, "A (référence)");
                            ui.selectable_value(&mut self.group, SessionGroup::B, "B (avec réglage)");
                        });
                    ui.label("Arrêt auto (min, 0 = manuel) :");
                    ui.add(egui::DragValue::new(&mut self.auto_stop_min).clamp_range(0..=240));
                    if ui.add(theme::primary_button("Démarrer la session")).clicked() {
                        start = true;
                    }
                });
            } else {
                ui.horizontal_wrapped(|ui| {
                    let el = self.started_at.map(|t| t.elapsed().as_secs()).unwrap_or(0);
                    ui.label(RichText::new(format!("Session en cours : {}", fmt_duration(el))).strong());
                    if !self.label.is_empty() {
                        theme::badge(ui, &self.label, theme::ACCENT);
                    }
                    if self.group != SessionGroup::None {
                        theme::badge(ui, &format!("Groupe {}", self.group.label()), theme::WARN);
                    }
                    if ui
                        .add(egui::Button::new(RichText::new(format!("Marquer un lag ({})", self.settings.hotkey_label())).strong()).fill(Color32::from_rgb(120, 90, 10)))
                        .clicked()
                    {
                        mark = true;
                    }
                    ui.label(theme::muted(&format!("{} marqueur(s)", marker_count)));
                    if ui.add(theme::danger_button("Arrêter et enregistrer")).clicked() {
                        stop = true;
                    }
                });
            }
        });
        if start {
            self.start_session();
        }
        if stop {
            self.stop_session(&ctx);
        }
        if mark {
            self.add_marker_now();
        }
        ui.add_space(8.0);

        let view = match self.view.clone() {
            Some(v) => v,
            None => {
                theme::card(ui, |ui| {
                    ui.label(RichText::new("Comment utiliser l'outil").strong());
                    ui.add_space(4.0);
                    ui.label("1. Lancez une session ici, puis jouez normalement (Minecraft peut être lancé avant ou après).");
                    ui.label(format!("2. Quand vous sentez un hit en retard, appuyez sur {} : l'instant est enregistré.", self.settings.hotkey_label()));
                    ui.label("3. Arrêtez la session, puis ouvrez « Diagnostic » : l'outil indique si le réseau explique vos ressentis, et où se situe le problème (Wi-Fi, ligne, route).");
                    ui.label("4. Pour tester un réglage : sessions A (sans) puis B (avec), au moins 2 de chaque, dans « Comparaison A/B ».");
                    ui.add_space(4.0);
                    ui.label(theme::muted("Pour lire le RTT et les retransmissions réels de la connexion Minecraft, relancez l'outil en administrateur (Paramètres)."));
                });
                return;
            }
        };

        // ---- Cartes de mesure ----
        ui.columns(4, |cols| {
            for (i, tv) in view.targets.iter().enumerate() {
                let reason = match tv.target {
                    Target::Gateway if view.gateway_ip.is_none() => Some("passerelle non détectée".to_string()),
                    Target::Server if view.server_target.is_none() => Some("serveur non résolu".to_string()),
                    Target::Game => Some(view.game.estats.clone()),
                    _ => None,
                };
                target_card(&mut cols[i], tv, reason);
            }
        });
        ui.add_space(8.0);

        // ---- Graphique ----
        theme::card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Latence en temps réel").strong());
                ui.label(theme::muted("fenêtre :"));
                let mut w = self.settings.chart_window_s;
                egui::ComboBox::from_id_source("cw").selected_text(format!("{} s", w)).show_ui(ui, |ui| {
                    for v in [60u32, 120, 300, 600] {
                        ui.selectable_value(&mut w, v, format!("{} s", v));
                    }
                });
                if w != self.settings.chart_window_s {
                    self.settings.chart_window_s = w;
                    self.save_settings();
                }
            });
            let series: Vec<ChartSeries> = view
                .targets
                .iter()
                .filter(|t| !t.points.is_empty())
                .map(|t| ChartSeries { name: t.target.short(), color: series_color(t.target), points: &t.points })
                .collect();
            let markers: Vec<u64> = view.markers.iter().map(|m| m.t_ms).collect();
            latency_chart(ui, 250.0, self.settings.chart_window_s as f64, view.now_ms, &series, &markers);
        });
        ui.add_space(8.0);

        // ---- Wi-Fi, jeu, marqueurs ----
        ui.columns(3, |cols| {
            theme::card(&mut cols[0], |ui| {
                ui.label(RichText::new("Wi-Fi").strong());
                match &view.wifi {
                    Some(w) => {
                        ui.label(format!("{} · {}", w.ssid, w.band));
                        ui.add(egui::ProgressBar::new(w.quality as f32 / 100.0).text(format!("signal {} %", w.quality)));
                        let mut line = format!("{} · canal {}", w.phy, w.channel.map(|c| c.to_string()).unwrap_or_else(|| "?".to_string()));
                        if let Some(r) = w.rssi_dbm {
                            line.push_str(&format!(" · {} dBm", r));
                        }
                        ui.label(theme::muted(&line));
                        ui.label(theme::muted(&format!("débit négocié : ↓ {:.0} / ↑ {:.0} Mbit/s", w.rx_mbps, w.tx_mbps)));
                        ui.label(theme::muted(&w.adapter));
                    }
                    None => {
                        ui.label(theme::muted("Aucune liaison Wi-Fi détectée (connexion filaire ?)"));
                    }
                }
            });
            theme::card(&mut cols[1], |ui| {
                ui.label(RichText::new("Minecraft").strong());
                if view.game.java_pids.is_empty() {
                    ui.label(theme::muted("javaw.exe non détecté"));
                } else {
                    theme::badge(ui, &format!("javaw.exe détecté ({})", view.game.java_pids.len()), theme::GOOD);
                }
                match &view.game.connection {
                    Some(c) => ui.label(theme::muted(c)),
                    None => ui.label(theme::muted("aucune connexion serveur")),
                };
                ui.label(theme::muted(&format!("mesure réelle : {}", view.game.estats)));
                if let Some((d, u)) = view.load_now {
                    ui.label(theme::muted(&format!("charge réseau du PC : ↓ {:.0} / ↑ {:.0} kbit/s", d, u)));
                }
                for n in &view.notes {
                    ui.label(RichText::new(n).color(theme::WARN).size(12.5));
                }
            });
            theme::card(&mut cols[2], |ui| {
                ui.label(RichText::new("Marqueurs").strong());
                if view.markers.is_empty() {
                    ui.label(theme::muted(&format!("Aucun. Appuyez sur {} quand vous sentez un lag.", self.settings.hotkey_label())));
                }
                for m in view.markers.iter().rev().take(6) {
                    ui.label(format!("• {}", fmt_time(m.t_ms, "%H:%M:%S")));
                }
            });
        });
    }
}
