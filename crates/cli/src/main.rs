//! Interface en ligne de commande de MC Net Optimizer.

use std::time::{Duration, Instant};

use mcnet_core::analysis::compare::compare_groups;
use mcnet_core::analysis::diagnosis::{diagnose, Severity};
use mcnet_core::model::{Session, SessionGroup, Target};
use mcnet_core::monitor::Monitor;
use mcnet_core::optimizer::{Applicability, Optimizer};
use mcnet_core::store::Store;
use mcnet_core::util::{fmt_duration, fmt_time};

fn usage() {
    println!(
        "MC Net Optimizer (CLI)

Utilisation :
  mcnet-cli monitor <secondes> [--label TEXTE] [--group A|B]   Mesure puis enregistre la session
  mcnet-cli sessions                                          Liste les sessions enregistrées
  mcnet-cli diagnose [ID|latest]                              Diagnostic d'une session
  mcnet-cli compare                                           Compare les sessions du groupe A et du groupe B
  mcnet-cli tweaks                                            État des optimisations
  mcnet-cli tweaks apply <ID>                                 Applique une optimisation (réversible)
  mcnet-cli tweaks revert <ID|all>                            Annule une optimisation
  mcnet-cli path                                              Dossier de données"
    );
}

fn open_store() -> Store {
    match Store::new(Store::default_dir()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Erreur : {}", e);
            std::process::exit(1);
        }
    }
}

fn print_diag(s: &Session) {
    let d = diagnose(s);
    println!("Session {} — durée {}", s.id, fmt_duration(d.duration_s as u64));
    for (t, st) in &d.stats {
        if st.has_data() {
            println!(
                "  {:<22} p50 {:>6.1} ms | p95 {:>6.1} | p99 {:>6.1} | gigue {:>5.1} | perte {:>5.2} % ({} échantillons)",
                t.label(), st.p50, st.p95, st.p99, st.jitter, st.loss_pct, st.total
            );
        } else {
            println!("  {:<22} aucune réponse ({} sondes)", t.label(), st.total);
        }
    }
    println!();
    for f in &d.findings {
        let tag = match f.severity {
            Severity::Ok => "[OK]  ",
            Severity::Info => "[INFO]",
            Severity::Warning => "[ATTN]",
            Severity::Critical => "[CRIT]",
        };
        println!("{} {}", tag, f.title);
        println!("       {}", f.detail);
        for e in &f.evidence {
            println!("       - {}", e);
        }
    }
    for m in &d.marker_reports {
        let verdict = if m.any_anomaly { "anomalie réseau coïncidente" } else { "réseau stable" };
        println!("Marqueur {} : {}", fmt_time(m.t_ms, "%H:%M:%S"), verdict);
    }
}

fn cmd_monitor(args: &[String]) {
    let secs: u64 = args.first().and_then(|s| s.parse().ok()).unwrap_or(60);
    let mut label = String::new();
    let mut group = SessionGroup::None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--label" if i + 1 < args.len() => {
                label = args[i + 1].clone();
                i += 1;
            }
            "--group" if i + 1 < args.len() => {
                group = match args[i + 1].to_uppercase().as_str() {
                    "A" => SessionGroup::A,
                    "B" => SessionGroup::B,
                    _ => SessionGroup::None,
                };
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    let store = open_store();
    let settings = store.load_settings();
    let optimizer = Optimizer::with_default_tweaks(&store.dir);
    let m = Monitor::start(&settings.monitor, label, group, optimizer.active_ids());
    println!("Mesure pendant {} s (Ctrl+C pour interrompre sans enregistrer)...", secs);
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(secs) {
        std::thread::sleep(Duration::from_secs(5));
        let v = m.view(30, 30);
        let mut line = format!("[{:>4}s]", t0.elapsed().as_secs());
        for tv in &v.targets {
            if tv.stats.has_data() {
                line.push_str(&format!("  {} p50 {:.1} / p95 {:.1}", tv.target.short(), tv.stats.p50, tv.stats.p95));
            }
        }
        println!("{}", line);
    }
    let session = m.stop();
    if let Err(e) = store.save_session(&session) {
        eprintln!("Enregistrement impossible : {}", e);
    } else {
        println!("Session enregistrée : {}", session.id);
    }
    println!();
    print_diag(&session);
}

fn cmd_diagnose(args: &[String]) {
    let store = open_store();
    let id = match args.first().map(|s| s.as_str()) {
        None | Some("latest") => match store.list().first() {
            Some(m) => m.id.clone(),
            None => {
                eprintln!("Aucune session enregistrée.");
                return;
            }
        },
        Some(x) => x.to_string(),
    };
    match store.load_session(&id) {
        Ok(s) => print_diag(&s),
        Err(e) => eprintln!("Erreur : {}", e),
    }
}

fn cmd_compare() {
    let store = open_store();
    let mut a: Vec<Session> = Vec::new();
    let mut b: Vec<Session> = Vec::new();
    for m in store.list() {
        if m.group == SessionGroup::None {
            continue;
        }
        if let Ok(s) = store.load_session(&m.id) {
            if s.group == SessionGroup::A {
                a.push(s);
            } else if s.group == SessionGroup::B {
                b.push(s);
            }
        }
    }
    println!("Groupe A : {} session(s) ; groupe B : {} session(s)", a.len(), b.len());
    let ar: Vec<&Session> = a.iter().collect();
    let br: Vec<&Session> = b.iter().collect();
    for c in compare_groups(&ar, &br) {
        println!(
            "{:<22} p50 A {:.1} → B {:.1} | p95 A {:.1} → B {:.1} | gigue A {:.1} → B {:.1} | perte A {:.2} % → B {:.2} %",
            c.target.label(), c.a.p50, c.b.p50, c.a.p95, c.b.p95, c.a.jitter, c.b.jitter, c.a.loss_pct, c.b.loss_pct
        );
        if let Some(m) = &c.mw {
            println!("    Mann-Whitney sur blocs de 30 s : p = {:.4}, delta de Cliff = {:.2}", m.p, m.cliffs_delta);
        }
        println!("    Verdict : {}", c.verdict.text());
    }
}

fn cmd_tweaks(args: &[String]) {
    let store = open_store();
    let opt = Optimizer::with_default_tweaks(&store.dir);
    for msg in opt.recover() {
        println!("{}", msg);
    }
    match args.first().map(|s| s.as_str()) {
        None => {
            for st in opt.status() {
                let state = if st.active {
                    "ACTIF".to_string()
                } else {
                    match &st.applicability {
                        Applicability::Applicable { current } => format!("applicable ({})", current),
                        Applicability::NotNeeded { reason } => format!("inutile : {}", reason),
                        Applicability::Unavailable { reason } => format!("indisponible : {}", reason),
                    }
                };
                println!("{} — {}\n    {}\n    Commande : {}\n    Risque : {}", st.info.id, state, st.info.title, st.plan, st.info.risk);
            }
        }
        Some("apply") => match args.get(1) {
            Some(id) => match opt.apply(id) {
                Ok(m) => println!("{}", m),
                Err(e) => eprintln!("Erreur : {}", e),
            },
            None => usage(),
        },
        Some("revert") => match args.get(1).map(|s| s.as_str()) {
            Some("all") => {
                for (id, r) in opt.revert_all() {
                    match r {
                        Ok(()) => println!("{} : annulé", id),
                        Err(e) => eprintln!("{} : {}", id, e),
                    }
                }
            }
            Some(id) => match opt.revert(id) {
                Ok(()) => println!("{} : annulé", id),
                Err(e) => eprintln!("Erreur : {}", e),
            },
            None => usage(),
        },
        _ => usage(),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(|s| s.as_str()) {
        Some("monitor") => cmd_monitor(&args[1..]),
        Some("sessions") => {
            let store = open_store();
            for m in store.list() {
                println!(
                    "{}  [{}]  {}  durée {}  {} échantillons  {} marqueurs",
                    m.id, m.group.label(), m.label, fmt_duration(m.duration_s as u64), m.sample_count, m.marker_count
                );
            }
        }
        Some("diagnose") => cmd_diagnose(&args[1..]),
        Some("compare") => cmd_compare(),
        Some("tweaks") => cmd_tweaks(&args[1..]),
        Some("path") => println!("{}", Store::default_dir().display()),
        _ => usage(),
    }
    let _ = Target::ALL;
}
