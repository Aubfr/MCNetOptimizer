//! Parametres et informations.

use eframe::egui::{self, RichText};

use crate::app::App;
use crate::theme;

impl App {
    pub fn ui_settings(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.heading("Paramètres");
        ui.add_space(8.0);

        let before = self.settings.clone();
        let mut hotkey_changed = false;
        let mut relaunch = false;
        let mut open_dir = false;

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            theme::card(ui, |ui| {
                ui.label(RichText::new("Sondes").strong());
                ui.horizontal(|ui| {
                    ui.label("Serveur sonde (si aucune connexion de jeu) :");
                    ui.add(egui::TextEdit::singleline(&mut self.settings.monitor.server_host).desired_width(190.0));
                    ui.add(egui::DragValue::new(&mut self.settings.monitor.server_port).clamp_range(1..=65535));
                });
                ui.checkbox(&mut self.settings.monitor.auto_detect_server, "Utiliser l'adresse réelle de la connexion de Minecraft quand elle est détectée");
                ui.horizontal(|ui| {
                    ui.label("Hôte Internet de référence (IPv4) :");
                    ui.add(egui::TextEdit::singleline(&mut self.settings.monitor.internet_ip).desired_width(130.0));
                });
                ui.horizontal(|ui| {
                    ui.label("Cadence ICMP (ms) :");
                    ui.add(egui::DragValue::new(&mut self.settings.monitor.icmp_interval_ms).clamp_range(500..=5000).speed(50.0));
                    ui.label("Cadence TCP serveur (ms, min 3000) :");
                    ui.add(egui::DragValue::new(&mut self.settings.monitor.tcp_interval_ms).clamp_range(3000..=60000).speed(250.0));
                });
                ui.label(theme::muted("La sonde TCP ouvre puis ferme une connexion vers le serveur à faible cadence (5 s par défaut). Gardez-la modérée : un serveur peut limiter les connexions répétées d'une même adresse."));
            });
            ui.add_space(8.0);

            theme::card(ui, |ui| {
                ui.label(RichText::new("Marqueur « ça a lag »").strong());
                ui.horizontal_wrapped(|ui| {
                    if ui.checkbox(&mut self.settings.hotkey_enabled, "Raccourci global").changed() {
                        hotkey_changed = true;
                    }
                    ui.label("Touche : F");
                    if ui.add(egui::DragValue::new(&mut self.settings.hotkey_fn).clamp_range(1..=12)).changed() {
                        hotkey_changed = true;
                    }
                    if ui.checkbox(&mut self.settings.hotkey_ctrl, "Ctrl").changed() {
                        hotkey_changed = true;
                    }
                    if ui.checkbox(&mut self.settings.hotkey_alt, "Alt").changed() {
                        hotkey_changed = true;
                    }
                    if ui.checkbox(&mut self.settings.hotkey_shift, "Maj").changed() {
                        hotkey_changed = true;
                    }
                    if ui.checkbox(&mut self.settings.beep_on_marker, "Bip de confirmation").changed() {
                        hotkey_changed = true;
                    }
                });
                ui.label(theme::muted(&format!("Raccourci actuel : {}. Il est réservé par Windows tant que l'outil tourne (la touche ne parvient plus aux autres programmes). L'outil ne lit aucune autre touche.", self.settings.hotkey_label())));
                if let Some(e) = &self.hotkey_error {
                    ui.label(RichText::new(e).color(theme::BAD));
                }
            });
            ui.add_space(8.0);

            theme::card(ui, |ui| {
                ui.label(RichText::new("Général").strong());
                ui.checkbox(&mut self.settings.revert_on_exit, "Annuler automatiquement les optimisations à la fermeture de l'outil");
                ui.horizontal(|ui| {
                    ui.label("Fenêtre des statistiques en direct (s) :");
                    ui.add(egui::DragValue::new(&mut self.settings.stats_window_s).clamp_range(10..=600));
                });
                ui.horizontal(|ui| {
                    if ui.button("Ouvrir le dossier des données").clicked() {
                        open_dir = true;
                    }
                    if !self.is_admin && ui.button("Relancer en administrateur").clicked() {
                        relaunch = true;
                    }
                });
                ui.label(theme::muted(&format!("Dossier : {}", self.store.dir.display())));
            });
            ui.add_space(8.0);

            theme::card(ui, |ui| {
                ui.label(RichText::new("À propos et limites").strong());
                ui.label(format!("MC Net Optimizer {}", env!("CARGO_PKG_VERSION")));
                for l in [
                    "L'outil observe : aucune capture, modification ou injection de paquets, aucun hook, aucun accès au contenu du jeu, aucun fichier de Lunar/Badlion touché.",
                    "Il ne peut pas mesurer le hit registration lui-même : seulement les conditions réseau autour de vos ressentis.",
                    "Ce qui dépend de la route de votre FAI, de la distance ou du serveur Hypixel ne peut pas être corrigé localement.",
                    "Minecraft 1.8.9 fonctionne à 20 ticks/s (50 ms) : cette quantification est indépendante du ping et incompressible.",
                    "Le RTT réel de la connexion de jeu nécessite les droits administrateur (statistiques TCP de Windows).",
                ] {
                    ui.label(theme::muted(&format!("• {}", l)));
                }
            });
        });

        if self.settings != before {
            self.save_settings();
        }
        if hotkey_changed {
            self.setup_hotkey(&ctx);
        }
        if relaunch {
            self.relaunch_admin(&ctx);
        }
        if open_dir {
            let dir = self.store.dir.clone();
            let _ = std::process::Command::new("explorer").arg(dir).spawn();
        }
    }
}
