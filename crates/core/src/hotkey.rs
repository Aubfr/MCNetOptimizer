//! Marqueur d'evenement via raccourci clavier global.
//!
//! Utilise `RegisterHotKey` : Windows previent notre thread quand la combinaison est
//! pressee. L'outil ne lit PAS les autres touches, n'installe AUCUN hook et n'interagit
//! pas avec le jeu. Le raccourci est reserve systeme-wide tant que l'outil tourne.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::platform;
use crate::util::now_ms;

pub struct HotkeyListener {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

pub const VK_F1: u32 = 0x70;

/// Touche de fonction Fn (1..=12) -> code virtuel.
pub fn vk_for_function_key(n: u32) -> u32 {
    VK_F1 + n.clamp(1, 12) - 1
}

impl HotkeyListener {
    /// Demarre l'ecoute. `on_press(t_ms)` est appele depuis le thread d'ecoute, avec
    /// l'horodatage exact de la pression.
    pub fn start(
        modifiers: u32,
        vk: u32,
        beep: bool,
        on_press: Box<dyn Fn(u64) + Send + 'static>,
    ) -> Result<HotkeyListener, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_t = stop.clone();
        let (tx, rx) = mpsc::channel::<Result<(), String>>();
        let handle = std::thread::spawn(move || {
            match platform::register_hotkey(modifiers, vk) {
                Ok(()) => {
                    let _ = tx.send(Ok(()));
                }
                Err(code) => {
                    let _ = tx.send(Err(format!(
                        "Raccourci indisponible (code {}). Il est peut-être déjà utilisé par un autre programme.",
                        code
                    )));
                    return;
                }
            }
            while !stop_t.load(Ordering::Relaxed) {
                if platform::poll_hotkey() {
                    let t = now_ms();
                    on_press(t);
                    if beep {
                        platform::beep(880, 70);
                    }
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            platform::unregister_hotkey();
        });
        match rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(())) => Ok(HotkeyListener { stop, handle: Some(handle) }),
            Ok(Err(e)) => {
                let _ = handle.join();
                Err(e)
            }
            Err(_) => {
                stop.store(true, Ordering::Relaxed);
                Err("Délai dépassé lors de l'enregistrement du raccourci.".to_string())
            }
        }
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for HotkeyListener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_key_mapping() {
        assert_eq!(vk_for_function_key(1), 0x70);
        assert_eq!(vk_for_function_key(8), 0x77);
        assert_eq!(vk_for_function_key(12), 0x7B);
        assert_eq!(vk_for_function_key(99), 0x7B);
    }
}
