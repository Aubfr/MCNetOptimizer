//! Page de diagnostic : causes probables, avec les chiffres a l'appui.

use eframe::egui::{self, Color32, RichText};
use mcnet_core::analysis::diagnosis::{diagnose, Severity};
use mcnet_core::util::{fmt_duration, fmt_time};

use crate::app::App;
use crate::theme;

fn sev(s: Severity) -> (&'static str, Color32) {
    match s {
        Severity::Ok => ("OK", theme::GOOD),
        Severity::Info => ("Info", theme::ACCENT),
        Severity::Warning => ("Attention", theme::WARN),
        Severity::Critical => ("Critique", theme::BAD),
    }
}

impl App {
    pub fn ui_diagnostic(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.heading("Diagnostic");
        ui.label(theme::muted(
            "Sépare ce qui est local (Wi-Fi, box) de ce qui ne l'est pas (ligne, route, serveur) et dit honnêtement quand le réseau n'explique pas le ressenti.",
        ));
        ui.add_space(8.0);

        let mut analyze_live = false;
        let mut analyze_saved: Option<String> = None;
        theme::card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if self.monitor.is_some() && ui.add(theme::primary_button("Analyser la session en cours")).clicked() {
                    analyze_live = true;
                }
                ui.label("Session enregistrée :");
                let sel = self.diag_choice.clone().unwrap_or_else(|| "choisir…".to_string());
                egui::ComboBox::from_id_source("dg").selected_text(sel).width(260.0).show_ui(ui, |ui| {
                    for m in &self.history {
                        let t = format!("{}  {}  [{}]", fmt_time(m.started_ms, "%d/%m %H:%M"), m.label, m.group.label());
                        ui.selectable_value(&mut self.diag_choice, Some(m.id.clone()), t);
                    }
                });
                if ui.button("Analyser").clicked() {
                    analyze_saved = self.diag_choice.clone();
                }
                if self.diag_task.running() {
                    ui.spinner();
                }
            });
        });

        if analyze_live {
            if let Some(m) = &self.monitor {
                let s = m.snapshot_session();
                self.diag_task.spawn(&ctx, move || {
                    let d = diagnose(&s);
                    Ok((s.id.clone(), d))
                });
            }
        }
        if let Some(id) = analyze_saved {
            let store = self.store.clone();
            self.diag_task.spawn(&ctx, move || {
                store.load_session(&id).map(|s| {
                    let d = diagnose(&s);
                    (s.id.clone(), d)
                })
            });
        }
        ui.add_space(8.0);

        let diag = self.diag.clone();
        let (id, d) = match diag {
            Some(x) => x,
            None => {
                ui.label(theme::muted("Aucune analyse affichée. Lancez-en une ci-dessus."));
                return;
            }
        };

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.label(RichText::new(format!("Session {} · durée {}", id, fmt_duration(d.duration_s as u64))).strong());
            ui.add_space(4.0);

            theme::card(ui, |ui| {
                egui::Grid::new("diag-stats").striped(true).num_columns(7).spacing([18.0, 6.0]).show(ui, |ui| {
                    for h in ["Cible", "Sondes", "Médiane", "p95", "p99", "Gigue", "Perte"] {
                        ui.label(theme::muted(h));
                    }
                    ui.end_row();
                    for (t, st) in &d.stats {
                        ui.label(t.label());
                        ui.label(format!("{}", st.total));
                        if st.has_data() {
                            ui.label(format!("{:.1} ms", st.p50));
                            ui.label(format!("{:.1} ms", st.p95));
                            ui.label(format!("{:.1} ms", st.p99));
                            let c = theme::quality_color(st.jitter, st.loss_pct);
                            ui.label(RichText::new(format!("{:.1} ms", st.jitter)).color(c));
                            ui.label(RichText::new(format!("{:.2} %", st.loss_pct)).color(c));
                        } else {
                            ui.label("—");
                            ui.label("—");
                            ui.label("—");
                            ui.label("—");
                            ui.label(RichText::new(format!("{:.0} %", st.loss_pct)).color(theme::BAD));
                        }
                        ui.end_row();
                    }
                });
            });
            ui.add_space(8.0);

            let mut findings = d.findings.clone();
            findings.sort_by(|a, b| b.severity.cmp(&a.severity));
            for f in &findings {
                let (label, color) = sev(f.severity);
                theme::card(ui, |ui| {
                    ui.horizontal(|ui| {
                        theme::badge(ui, label, color);
                        ui.label(RichText::new(&f.title).strong().size(15.5));
                    });
                    ui.label(&f.detail);
                    for e in &f.evidence {
                        ui.label(theme::muted(&format!("• {}", e)));
                    }
                });
                ui.add_space(6.0);
            }

            if !d.marker_reports.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new("Vos marqueurs « ça a lag »").strong().size(16.0));
                ui.label(theme::muted(
                    "Pour chaque marqueur : le réseau a-t-il montré un pic ou une perte dans les 8 s précédentes et la seconde suivante ?",
                ));
                theme::card(ui, |ui| {
                    for m in &d.marker_reports {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(fmt_time(m.t_ms, "%H:%M:%S")).strong());
                            if m.any_anomaly {
                                theme::badge(ui, "anomalie réseau", theme::WARN);
                            } else {
                                theme::badge(ui, "réseau stable", theme::GOOD);
                            }
                            for p in &m.per_target {
                                let txt = format!(
                                    "{} : max {:.0} ms{}",
                                    p.target.short(),
                                    p.max_rtt,
                                    if p.lost > 0 { format!(", {} perte(s)", p.lost) } else { String::new() }
                                );
                                let c = if p.anomalous { theme::WARN } else { theme::MUTED };
                                ui.label(RichText::new(txt).color(c).size(12.5));
                            }
                        });
                    }
                });
            }
            ui.add_space(10.0);
            ui.label(theme::muted(
                "Limite : l'outil mesure les conditions réseau autour de vos ressentis, pas le hit registration lui-même. Une corrélation n'est pas une preuve.",
            ));
        });
    }
}
