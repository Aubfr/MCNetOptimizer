#![windows_subsystem = "windows"]
//! MC Net Optimizer — diagnostic et optimisation reseau reversible pour Minecraft 1.8.9.

mod app;
mod chart;
mod pages;
mod task;
mod theme;

use eframe::egui;
use mcnet_core::store::Store;

fn install_panic_log() {
    let dir = Store::default_dir();
    std::panic::set_hook(Box::new(move |info| {
        let _ = std::fs::create_dir_all(&dir);
        let msg = format!("{}\n", info);
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("crash.log"))
            .and_then(|mut f| std::io::Write::write_all(&mut f, msg.as_bytes()));
    }));
}

fn main() -> eframe::Result<()> {
    install_panic_log();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("MC Net Optimizer")
            .with_inner_size([1220.0, 780.0])
            .with_min_inner_size([980.0, 620.0]),
        ..Default::default()
    };
    eframe::run_native(
        "MC Net Optimizer",
        options,
        Box::new(|cc| {
            theme::apply(&cc.egui_ctx);
            Box::new(app::App::new(cc))
        }),
    )
}
