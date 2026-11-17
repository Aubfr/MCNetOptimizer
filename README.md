# MC Net Optimizer

Outil Windows 11 de **diagnostic et d'optimisation réseau réversible** pour Minecraft Java 1.8.9
(Hypixel, Lunar Client, Badlion Client). Écrit en Rust, interface egui.

> **Honnêteté d'abord.** L'outil mesure les conditions réseau autour de vos combats (latence, gigue,
> pertes, retransmissions TCP, Wi-Fi) et vous dit si elles expliquent ce que vous ressentez.
> Il **ne mesure pas** le hit registration lui-même et **ne promet aucune amélioration**.
> Si le problème vient de la route de votre FAI, de la distance ou du serveur, il le dit et ne
> prétend pas pouvoir le corriger.

## État de vérification du code (à lire)

| Élément | État |
|---|---|
| Code source complet (core, CLI, GUI) | Écrit |
| Compilation | **Non vérifiée par l'auteur** : l'environnement de développement n'avait ni Rust ni accès réseau |
| Tests automatisés (76) | Écrits, **jamais exécutés par l'auteur** ; ils s'exécutent sur Windows dans GitHub Actions |
| Fonctions Windows (ICMP, table TCP, EStats, WLAN, raccourci global, priorité) | Implémentées par FFI, **jamais lancées sur un vrai Windows** |
| Lecture du RTT réel (EStats) | Dépend de la structure de Windows ; protégée par des contrôles de cohérence, mode dégradé sinon |

Un premier passage en CI peut donc révéler des erreurs de compilation ou des tests en échec : le
workflow publie les journaux (`build.log`, `test.log`) pour les corriger rapidement.

## Obtenir le .exe (une seule action)

1. Créez un dépôt GitHub vide, puis envoyez-y **le contenu de ce dossier** (bouton *Add file → Upload files*,
   glisser-déposer tout le contenu, y compris le dossier `.github`).
2. GitHub Actions compile, teste et publie automatiquement.
3. Téléchargez `MCNetOptimizer-windows-x64.zip` dans l'onglet **Releases** (version « latest »),
   ou dans l'onglet **Actions → dernier run → Artifacts** (qui contient aussi les journaux).

Si l'onglet Actions n'affiche aucun workflow (dossier `.github` ignoré à l'envoi) : créez le fichier
`.github/workflows/build.yml` via *Add file → Create new file* et collez-y le contenu de `ci/build.yml`.

En cas d'erreur de compilation, ouvrez le run échoué, copiez le journal `build.log` (ou `test.log`) et
transmettez-le pour correction.

Windows SmartScreen peut avertir (exécutable non signé) : *Informations complémentaires → Exécuter quand même*.

## Utilisation

1. Lancez `MCNetOptimizer.exe` (idéalement **en administrateur** pour lire le RTT réel de la connexion Minecraft).
2. **Tableau de bord → Démarrer la session**, puis jouez normalement.
3. Quand un hit semble en retard, appuyez sur **F8** (modifiable) : l'instant est enregistré.
4. Arrêtez la session, ouvrez **Diagnostic** : causes probables, avec chiffres, et corrélation avec vos marqueurs.
5. **Optimisations** : réglages réversibles proposés seulement s'ils sont pertinents sur votre machine.
6. Pour prouver qu'un réglage sert à quelque chose : sessions A (sans) / B (avec) puis **Comparaison A/B**.

Données : `%APPDATA%\McNetOptimizer` (sessions JSON, paramètres, journal des réglages, exports CSV, `crash.log`).

Ligne de commande : `mcnet-cli monitor 600 --label test --group A`, `mcnet-cli diagnose latest`,
`mcnet-cli compare`, `mcnet-cli tweaks`, `mcnet-cli tweaks apply <id>`, `mcnet-cli tweaks revert all`.

## Ce que l'outil mesure

| Cible | Méthode | Ce que ça isole |
|---|---|---|
| Box (passerelle) | ICMP 1/s | Wi-Fi et réseau local |
| Internet (1.1.1.1) | ICMP 1/s | Ligne / FAI, bufferbloat |
| Serveur | connexion TCP ouverte puis fermée, 1 / 5 s (adresse réelle de javaw si détectée) | Route vers le serveur |
| Connexion de jeu | RTT et retransmissions réels de la connexion de `javaw.exe` (statistiques TCP de Windows, sans lire le contenu) | Ce que vit réellement Minecraft |
| Wi-Fi | API WLAN native : signal, canal, bande, débits, changements de borne | Cause locale la plus fréquente |
| Charge locale | compteurs d'interfaces | Corrélation charge ↔ latence (bufferbloat) |

Statistiques : médiane, p95, p99, gigue, pertes isolées vs en rafale, pics et périodicité (scan Wi-Fi, tâches récurrentes).
Comparaison A/B : blocs de 30 s, test de Mann-Whitney, delta de Cliff ; une amélioration n'est déclarée que
si p < 0,01, effet non négligeable, ≥ 1 ms d'écart de p95 et ≥ 2 sessions par groupe.

## Optimisations (toutes réversibles, aucune sans votre clic)

| Id | Action | Admin |
|---|---|---|
| `wifi_plan_power` | Économie d'énergie Wi-Fi du plan d'alimentation → performances maximales (secteur) | non |
| `wifi_adapter_power` | Empêche Windows d'éteindre la carte Wi-Fi pour économiser l'énergie (peut couper le Wi-Fi quelques secondes) | oui |
| `java_priority` | `javaw.exe` en priorité « supérieure à la normale » (jamais haute/temps réel) | non |

Cycle : détection → snapshot → journal → application → vérification → (échec = restauration automatique).
Au démarrage, un réglage resté « en cours » après un plantage est annulé automatiquement.

**Refusé volontairement** : réglages TCP/registre « magiques » (placebos pour Minecraft), toute interception,
modification ou temporisation de paquets, désactivation de Defender/pare-feu/services, modification des fichiers
de Lunar ou Badlion, réglages irréversibles.

## Sécurité et conformité

- Observation seule : pas de capture de paquets (ni Npcap ni WinDivert), pas de pilote, pas d'injection, pas d'overlay,
  pas de lecture de la mémoire du jeu, pas de hook clavier (le marqueur utilise `RegisterHotKey`).
- Aucun avantage de jeu ni modification du gameplay.
- La sonde TCP vers le serveur est à faible cadence ; ne la réduisez pas excessivement (limites de connexions côté serveur).

## Limites techniques

- Minecraft 1.8.9 tourne à 20 ticks/s (50 ms) : quantification incompressible, indépendante du ping.
- Minecraft utilise TCP : une perte provoque un gel (head-of-line blocking), souvent plus gênant qu'un ping élevé.
- Route, distance, peering du FAI : hors de portée d'un logiciel local.
- Le vrai remède au bufferbloat est un SQM (fq_codel/CAKE) sur le routeur.
- Connexions IPv6 de jeu non couvertes (Java utilise IPv4 par défaut).
- Bande Wi-Fi déduite du canal (5 GHz et 6 GHz regroupés).

## Architecture

```
crates/core   mesure, analyse, diagnostic, optimiseur, persistance (bibliothèque)
  analysis/   stats, pics, comparaison A/B, diagnostic (pur, testable)
  platform/   FFI Windows (iphlpapi, wlanapi, kernel32, user32, shell32)
  monitor.rs  threads de mesure + instantané pour l'UI
  optimizer/  journal transactionnel + 3 réglages
crates/cli    outil en ligne de commande
crates/gui    interface egui (eframe 0.27)
```

Choix techniques : Rust (binaire unique, sans GC), FFI direct plutôt que la crate `windows` (moins de dépendances,
pas de rupture entre versions), historique en JSON plutôt que SQLite (aucun code C à compiler).

## Développement

```
cargo test --workspace
cargo build --release --workspace
```
Windows uniquement. Les tests d'intégration lancent de vraies sondes (loopback, ICMP, `powercfg`, PowerShell).
