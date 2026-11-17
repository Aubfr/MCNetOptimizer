//! Optimisations reversibles.

use eframe::egui::{self, RichText};
use mcnet_core::optimizer::Applicability;
use mcnet_core::platform;

use crate::app::App;
use crate::theme;

impl App {
    pub fn relaunch_admin(&mut self, ctx: &egui::Context) {
        if self.monitor.is_some() {
            self.toast("Arrêtez d'abord la session en cours (elle serait perdue), puis relancez en administrateur.", false);
            return;
        }
        if platform::relaunch_as_admin() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            self.toast("Élévation refusée ou impossible.", false);
        }
    }

    pub fn ui_tweaks(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if !self.tweaks_loaded && !self.tweaks_task.running() {
            self.refresh_tweaks(&ctx);
        }
        ui.heading("Optimisations");
        ui.label(theme::muted(
            "Chaque réglage est détecté avant d'être proposé, vérifié après application et annulable. Rien n'est appliqué sans votre clic. Mesurez toujours avant/après (Comparaison A/B) : un réglage n'est utile que si la mesure le démontre.",
        ));
        ui.add_space(8.0);

        let mut relaunch = false;
        let mut refresh = false;
        let mut revert_all = false;
        let busy = self.tweak_action.running();

        if !self.is_admin {
            theme::card(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    theme::badge(ui, "Sans droits administrateur", theme::WARN);
                    ui.label("Certains réglages et la lecture du RTT réel de la connexion de jeu exigent les droits administrateur.");
                    if ui.button("Relancer en administrateur").clicked() {
                        relaunch = true;
                    }
                });
            });
            ui.add_space(8.0);
        }

        ui.horizontal(|ui| {
            if ui.button("Rafraîchir").clicked() {
                refresh = true;
            }
            if ui.add_enabled(!busy, theme::danger_button("Tout annuler")).clicked() {
                revert_all = true;
            }
            if busy || self.tweaks_task.running() {
                ui.spinner();
                ui.label(theme::muted("opération en cours…"));
            }
        });
        ui.add_space(6.0);

        let mut apply_id: Option<String> = None;
        let mut revert_id: Option<String> = None;
        let list = self.tweaks.clone();

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for st in &list {
                theme::card(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(st.info.title).strong().size(16.0));
                        if st.active {
                            theme::badge(ui, "ACTIF", theme::GOOD);
                        } else {
                            match &st.applicability {
                                Applicability::Applicable { .. } => theme::badge(ui, "applicable", theme::ACCENT),
                                Applicability::NotNeeded { .. } => theme::badge(ui, "inutile", theme::MUTED),
                                Applicability::Unavailable { .. } => theme::badge(ui, "indisponible", theme::WARN),
                            }
                        }
                        if st.info.requires_admin {
                            theme::badge(ui, "admin", theme::MUTED);
                        }
                    });
                    ui.label(st.info.what);
                    ui.label(theme::muted(&format!("Pourquoi : {}", st.info.why)));
                    ui.label(theme::muted(&format!("Risque : {}", st.info.risk)));
                    match &st.applicability {
                        Applicability::Applicable { current } => {
                            ui.label(theme::muted(&format!("État actuel : {}", current)));
                        }
                        Applicability::NotNeeded { reason } | Applicability::Unavailable { reason } => {
                            ui.label(theme::muted(&format!("État : {}", reason)));
                        }
                    }
                    egui::CollapsingHeader::new("Commande exécutée").id_source(st.info.id).show(ui, |ui| {
                        ui.label(RichText::new(&st.plan).monospace().size(12.5));
                    });
                    ui.horizontal(|ui| {
                        if st.active {
                            if ui.add_enabled(!busy, egui::Button::new("Annuler ce réglage")).clicked() {
                                revert_id = Some(st.info.id.to_string());
                            }
                        } else {
                            let can = matches!(st.applicability, Applicability::Applicable { .. }) && !busy;
                            if ui.add_enabled(can, theme::primary_button("Appliquer")).clicked() {
                                apply_id = Some(st.info.id.to_string());
                            }
                        }
                    });
                });
                ui.add_space(6.0);
            }

            ui.add_space(8.0);
            egui::CollapsingHeader::new("Ce que l'outil refuse volontairement de faire").show(ui, |ui| {
                for line in [
                    "Réglages TCP « magiques » (TcpAckFrequency, TCPNoDelay, MTU, etc.) : Minecraft active déjà TCP_NODELAY ; ce sont des placebos pour ce jeu.",
                    "Modifier ou retarder des paquets Minecraft, les capturer, les injecter : interdit par les règles des serveurs et sans bénéfice légitime.",
                    "Désactiver Windows Defender, le pare-feu ou des services : dangereux et sans lien avec la latence de jeu.",
                    "Modifier le registre de façon générique ou appliquer un réglage irréversible.",
                    "Toucher aux fichiers de Lunar Client ou Badlion Client.",
                    "Promettre une amélioration du hit registration : l'outil ne mesure que les conditions réseau.",
                ] {
                    ui.label(format!("• {}", line));
                }
            });
        });

        if relaunch {
            self.relaunch_admin(&ctx);
        }
        if refresh {
            self.refresh_tweaks(&ctx);
        }
        if let Some(id) = apply_id {
            let opt = self.optimizer.clone();
            self.tweak_action.spawn(&ctx, move || opt.apply(&id));
        }
        if let Some(id) = revert_id {
            let opt = self.optimizer.clone();
            self.tweak_action.spawn(&ctx, move || opt.revert(&id).map(|_| format!("Réglage « {} » annulé : état d'origine restauré.", id)));
        }
        if revert_all {
            let opt = self.optimizer.clone();
            self.tweak_action.spawn(&ctx, move || {
                let errs: Vec<String> = opt
                    .revert_all()
                    .into_iter()
                    .filter_map(|(id, r)| r.err().map(|e| format!("{} : {}", id, e)))
                    .collect();
                if errs.is_empty() {
                    Ok("Tous les réglages ont été annulés.".to_string())
                } else {
                    Err(errs.join(" ; "))
                }
            });
        }
    }
}
