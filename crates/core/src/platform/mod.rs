//! Couche Windows native. Uniquement des API systeme documentees, en lecture
//! (sauf la priorite de processus, reversible). Aucune lecture de la memoire du jeu,
//! aucun hook clavier, aucun pilote.

pub mod win;
pub use win::*;
