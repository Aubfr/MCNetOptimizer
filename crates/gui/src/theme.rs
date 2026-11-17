//! Theme sombre, composants reutilisables.

use eframe::egui::{self, Color32, FontId, Rounding, Stroke, TextStyle};

pub const BG: Color32 = Color32::from_rgb(13, 17, 23);
pub const SIDEBAR: Color32 = Color32::from_rgb(10, 13, 18);
pub const CARD: Color32 = Color32::from_rgb(22, 27, 34);
pub const BORDER: Color32 = Color32::from_rgb(40, 47, 58);
pub const TEXT: Color32 = Color32::from_rgb(230, 237, 243);
pub const MUTED: Color32 = Color32::from_rgb(139, 148, 158);
pub const ACCENT: Color32 = Color32::from_rgb(56, 139, 253);
pub const GOOD: Color32 = Color32::from_rgb(63, 185, 80);
pub const WARN: Color32 = Color32::from_rgb(227, 160, 8);
pub const BAD: Color32 = Color32::from_rgb(248, 81, 73);
pub const CHART_BG: Color32 = Color32::from_rgb(16, 20, 27);
pub const GRID: Color32 = Color32::from_rgb(33, 40, 50);

pub fn apply(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = CARD;
    v.extreme_bg_color = CHART_BG;
    v.override_text_color = Some(TEXT);
    v.widgets.noninteractive.bg_fill = CARD;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.inactive.bg_fill = Color32::from_rgb(33, 38, 45);
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(33, 38, 45);
    v.widgets.hovered.bg_fill = Color32::from_rgb(48, 54, 61);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(48, 54, 61);
    v.widgets.active.bg_fill = ACCENT;
    v.widgets.active.weak_bg_fill = ACCENT;
    v.selection.bg_fill = Color32::from_rgb(31, 78, 140);
    let r = Rounding::same(7.0);
    v.widgets.noninteractive.rounding = r;
    v.widgets.inactive.rounding = r;
    v.widgets.hovered.rounding = r;
    v.widgets.active.rounding = r;
    ctx.set_visuals(v);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.text_styles.insert(TextStyle::Heading, FontId::proportional(22.0));
    style.text_styles.insert(TextStyle::Body, FontId::proportional(14.5));
    style.text_styles.insert(TextStyle::Button, FontId::proportional(14.5));
    style.text_styles.insert(TextStyle::Small, FontId::proportional(12.0));
    ctx.set_style(style);
}

/// Carte arrondie.
pub fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::none()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .rounding(10.0)
        .inner_margin(egui::Margin::same(14.0))
        .show(ui, add)
        .inner
}

pub fn muted(text: &str) -> egui::RichText {
    egui::RichText::new(text).color(MUTED).size(12.5)
}

pub fn badge(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::none()
        .fill(color.gamma_multiply(0.22))
        .rounding(20.0)
        .inner_margin(egui::Margin::symmetric(9.0, 2.0))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).color(color).size(12.5).strong());
        });
}

pub fn primary_button(text: &str) -> impl egui::Widget {
    egui::Button::new(egui::RichText::new(text).strong().color(Color32::WHITE)).fill(ACCENT)
}

pub fn danger_button(text: &str) -> impl egui::Widget {
    egui::Button::new(egui::RichText::new(text).strong().color(Color32::WHITE)).fill(Color32::from_rgb(180, 50, 45))
}

/// Couleur selon gigue et perte (seuils prudents pour un jeu en TCP).
pub fn quality_color(jitter: f64, loss_pct: f64) -> Color32 {
    if jitter < 3.0 && loss_pct < 0.2 {
        GOOD
    } else if jitter < 8.0 && loss_pct < 1.0 {
        WARN
    } else {
        BAD
    }
}
