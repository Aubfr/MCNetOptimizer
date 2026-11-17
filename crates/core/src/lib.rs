//! mcnet-core : mesure, analyse et optimisation reversible des conditions reseau
//! pour Minecraft Java 1.8.9 sous Windows.
//!
//! Principe directeur : observer, jamais intercepter. Aucun paquet du jeu n'est
//! capture, modifie ou injecte ; aucune interaction avec Lunar/Badlion.

pub mod analysis;
pub mod config;
pub mod hotkey;
pub mod model;
pub mod monitor;
pub mod optimizer;
pub mod parse;
pub mod platform;
pub mod probe;
pub mod store;
pub mod util;
