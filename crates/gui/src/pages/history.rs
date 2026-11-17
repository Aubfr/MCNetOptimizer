//! Historique des sessions.

use eframe::egui::{self, RichText};
use mcnet_core::model::SessionGroup;
use mcnet_core::analysis::diagnosis::diagnose;
use mcnet_core::util::{fmt_duration, fmt_time};

use crate::app::{App, Page};
use crate::theme;

impl App {
    pub fn ui_history(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.heading("Historique");
        ui.label(theme::muted(&format!(
            "Sessions enregistrées dans {}",
            self.store.dir.join("sessions").display()
        )));
        ui.add_space(8.0);

        let mut select: Option<String> = None;
        let mut analyze: Option<String> = None;
        let mut delete: Option<String> = None;
        let mut export: Option<String> = None;
        let mut set_group: Option<(String, SessionGroup)> = None;
        let mut rename: Option<String> = None;
        let mut refresh = false;

        ui.horizontal(|ui| {
            if ui.button("Rafraîchir").clicked() {
                refresh = true;
            }
            ui.label(theme::muted(&format!("{} session(s)", self.history.len())));
        });
        ui.add_space(4.0);

        if let Some(sel) = self.selected.clone() {
            if self.history.iter().any(|m| m.id == sel) {
                theme::card(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(format!("Session {}", sel)).strong());
                        ui.add(egui::TextEdit::singleline(&mut self.rename_buf).desired_width(180.0).hint_text("libellé"));
                        if ui.button("Renommer").clicked() {
                            rename = Some(sel.clone());
                        }
                        if ui.button("Groupe A").clicked() {
                            set_group = Some((sel.clone(), SessionGroup::A));
                        }
                        if ui.button("Groupe B").clicked() {
                            set_group = Some((sel.clone(), SessionGroup::B));
                        }
                        if ui.button("Sans groupe").clicked() {
                            set_group = Some((sel.clone(), SessionGroup::None));
                        }
                        if ui.add(theme::primary_button("Diagnostic")).clicked() {
                            analyze = Some(sel.clone());
                        }
                        if ui.button("Exporter en CSV").clicked() {
                            export = Some(sel.clone());
                        }
                        if ui.add(theme::danger_button("Supprimer")).clicked() {
                            delete = Some(sel.clone());
                        }
                    });
                });
                ui.add_space(6.0);
            }
        }

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            theme::card(ui, |ui| {
                egui::Grid::new("hist").striped(true).num_columns(7).spacing([16.0, 6.0]).show(ui, |ui| {
                    for h in ["Date", "Libellé", "Groupe", "Durée", "Sondes", "Marqueurs", "Réglages actifs"] {
                        ui.label(theme::muted(h));
                    }
                    ui.end_row();
                    for m in &self.history {
                        let selected = self.selected.as_deref() == Some(m.id.as_str());
                        if ui.selectable_label(selected, fmt_time(m.started_ms, "%d/%m/%Y %H:%M")).clicked() {
                            select = Some(m.id.clone());
                        }
                        ui.label(&m.label);
                        ui.label(m.group.label());
                        ui.label(fmt_duration(m.duration_s as u64));
                        ui.label(format!("{}", m.sample_count));
                        ui.label(format!("{}", m.marker_count));
                        ui.label(if m.tweaks.is_empty() { "-".to_string() } else { m.tweaks.join(", ") });
                        ui.end_row();
                    }
                });
            });
        });

        if refresh {
            self.refresh_history();
        }
        if let Some(id) = select {
            self.rename_buf = self.history.iter().find(|m| m.id == id).map(|m| m.label.clone()).unwrap_or_default();
            self.selected = Some(id);
        }
        if let Some(id) = rename {
            let group = self.history.iter().find(|m| m.id == id).map(|m| m.group).unwrap_or(SessionGroup::None);
            match self.store.update_label(&id, self.rename_buf.trim(), group) {
                Ok(()) => self.toast("Libellé mis à jour.", true),
                Err(e) => self.toast(e, false),
            }
            self.refresh_history();
        }
        if let Some((id, g)) = set_group {
            let label = self.history.iter().find(|m| m.id == id).map(|m| m.label.clone()).unwrap_or_default();
            match self.store.update_label(&id, &label, g) {
                Ok(()) => self.toast(format!("Session classée dans le groupe {}.", g.label()), true),
                Err(e) => self.toast(e, false),
            }
            self.refresh_history();
        }
        if let Some(id) = analyze {
            let store = self.store.clone();
            self.diag_task.spawn(&ctx, move || {
                store.load_session(&id).map(|s| {
                    let d = diagnose(&s);
                    (s.id.clone(), d)
                })
            });
            self.page = Page::Diagnostic;
        }
        if let Some(id) = export {
            match self.store.load_session(&id).and_then(|s| self.store.export_csv(&s)) {
                Ok(p) => self.toast(format!("Exporté : {}", p.display()), true),
                Err(e) => self.toast(e, false),
            }
        }
        if let Some(id) = delete {
            match self.store.delete(&id) {
                Ok(()) => {
                    self.selected = None;
                    self.toast("Session supprimée.", true);
                }
                Err(e) => self.toast(e, false),
            }
            self.refresh_history();
        }
    }
}
