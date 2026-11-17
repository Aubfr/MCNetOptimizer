//! Comparaison A/B statistique.

use eframe::egui::{self, Color32, RichText};
use mcnet_core::analysis::compare::{compare_groups, Verdict};
use mcnet_core::model::{Session, SessionGroup};

use crate::app::App;
use crate::theme;

impl App {
    pub fn ui_compare(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.heading("Comparaison A/B");
        ui.label(theme::muted(
            "Prouve (ou non) qu'un réglage change vraiment la qualité de la connexion, en comparant des sessions sans (A) et avec (B) le réglage.",
        ));
        ui.add_space(8.0);

        let n_a = self.history.iter().filter(|m| m.group == SessionGroup::A).count();
        let n_b = self.history.iter().filter(|m| m.group == SessionGroup::B).count();

        theme::card(ui, |ui| {
            ui.label(RichText::new("Protocole recommandé").strong());
            ui.label("1. Lancez 2 sessions A (sans réglage), 2 sessions B (avec), en alternant A, B, A, B, de 10 à 20 minutes, au même moment de la journée et dans les mêmes conditions de jeu.");
            ui.label("2. Classez-les dans « Historique » (ou choisissez le groupe avant de démarrer).");
            ui.label("3. Comparez : le verdict exige au moins 2 sessions par groupe, 20 blocs de 30 s, p < 0,01, un effet non négligeable et un écart de p95 d'au moins 1 ms.");
            ui.add_space(4.0);
            ui.label(theme::muted("Une baisse du ping affiché ne prouve rien sur le hit registration : seule la stabilité (gigue, p95, pertes) est évaluée."));
        });
        ui.add_space(8.0);

        let mut run = false;
        ui.horizontal(|ui| {
            ui.label(format!("Groupe A : {} session(s) · Groupe B : {} session(s)", n_a, n_b));
            if ui.add_enabled(n_a > 0 && n_b > 0 && !self.cmp_task.running(), theme::primary_button("Comparer A et B")).clicked() {
                run = true;
            }
            if self.cmp_task.running() {
                ui.spinner();
            }
        });
        if run {
            let store = self.store.clone();
            let ids_a: Vec<String> = self.history.iter().filter(|m| m.group == SessionGroup::A).map(|m| m.id.clone()).collect();
            let ids_b: Vec<String> = self.history.iter().filter(|m| m.group == SessionGroup::B).map(|m| m.id.clone()).collect();
            self.cmp_task.spawn(&ctx, move || {
                let mut a: Vec<Session> = Vec::new();
                let mut b: Vec<Session> = Vec::new();
                for id in &ids_a {
                    a.push(store.load_session(id)?);
                }
                for id in &ids_b {
                    b.push(store.load_session(id)?);
                }
                let ar: Vec<&Session> = a.iter().collect();
                let br: Vec<&Session> = b.iter().collect();
                Ok((compare_groups(&ar, &br), a.len(), b.len()))
            });
        }
        ui.add_space(8.0);

        let cmp = self.cmp.clone();
        let (results, na, nb) = match cmp {
            Some(x) => x,
            None => return,
        };
        ui.label(theme::muted(&format!("Résultat sur {} session(s) A et {} session(s) B.", na, nb)));
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for c in &results {
                theme::card(ui, |ui| {
                    let (vtxt, vcol) = match &c.verdict {
                        Verdict::BetterB => (c.verdict.text(), theme::GOOD),
                        Verdict::WorseB => (c.verdict.text(), theme::BAD),
                        Verdict::NoDifference => (c.verdict.text(), theme::WARN),
                        Verdict::Insufficient(_) => (c.verdict.text(), theme::MUTED),
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(c.target.label()).strong().size(16.0));
                        theme::badge(ui, &vtxt, vcol);
                    });
                    egui::Grid::new(format!("cmp-{}", c.target.short())).striped(true).num_columns(4).spacing([22.0, 5.0]).show(ui, |ui| {
                        for h in ["", "A", "B", "Écart (B − A)"] {
                            ui.label(theme::muted(h));
                        }
                        ui.end_row();
                        let rows: [(&str, f64, f64, &str); 4] = [
                            ("Médiane (ms)", c.a.p50, c.b.p50, "ms"),
                            ("p95 (ms)", c.a.p95, c.b.p95, "ms"),
                            ("Gigue (ms)", c.a.jitter, c.b.jitter, "ms"),
                            ("Perte (%)", c.a.loss_pct, c.b.loss_pct, "%"),
                        ];
                        for (name, a, b, unit) in rows {
                            ui.label(name);
                            ui.label(format!("{:.2}", a));
                            ui.label(format!("{:.2}", b));
                            let d = b - a;
                            let col: Color32 = if d < -0.0001 { theme::GOOD } else if d > 0.0001 { theme::BAD } else { theme::MUTED };
                            ui.label(RichText::new(format!("{:+.2} {}", d, unit)).color(col));
                            ui.end_row();
                        }
                    });
                    let mut line = format!("Blocs de 30 s : A {} / B {}", c.blocks_a, c.blocks_b);
                    if let Some(m) = &c.mw {
                        line.push_str(&format!(" · Mann-Whitney p = {:.4} · effet (delta de Cliff) = {:.2}", m.p, m.cliffs_delta));
                    }
                    line.push_str(&format!(" · paires où B est plus rapide : {}/{}", c.pairs_b_faster.0, c.pairs_b_faster.1));
                    ui.label(theme::muted(&line));
                });
                ui.add_space(6.0);
            }
        });
    }
}
