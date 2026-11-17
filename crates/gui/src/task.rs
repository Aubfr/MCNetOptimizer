//! Petit utilitaire : executer un travail bloquant dans un thread et recuperer le resultat
//! sans geler l'interface.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui;

pub struct Task<T> {
    slot: Arc<Mutex<Option<T>>>,
    running: Arc<AtomicBool>,
}

impl<T: Send + 'static> Task<T> {
    pub fn new() -> Task<T> {
        Task { slot: Arc::new(Mutex::new(None)), running: Arc::new(AtomicBool::new(false)) }
    }

    pub fn running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    pub fn spawn<F>(&self, ctx: &egui::Context, f: F)
    where
        F: FnOnce() -> T + Send + 'static,
    {
        if self.running() {
            return;
        }
        self.running.store(true, Ordering::Relaxed);
        let slot = self.slot.clone();
        let running = self.running.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = f();
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
            running.store(false, Ordering::Relaxed);
            ctx.request_repaint();
        });
    }

    pub fn take(&self) -> Option<T> {
        self.slot.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}
