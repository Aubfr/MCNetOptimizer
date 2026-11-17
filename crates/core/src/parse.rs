//! Parseurs de sorties d'outils Windows (route, powercfg). Independants de la langue :
//! ils ne s'appuient que sur des motifs numeriques (adresses IP, GUID, valeurs 0x...).

use std::net::Ipv4Addr;

/// Passerelle par defaut d'apres `route print -4` (route 0.0.0.0/0 de plus faible metrique).
pub fn parse_route_print(output: &str) -> Option<Ipv4Addr> {
    let mut best: Option<(u32, Ipv4Addr)> = None;
    for line in output.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        if t.len() < 5 || t[0] != "0.0.0.0" || t[1] != "0.0.0.0" {
            continue;
        }
        let gw: Ipv4Addr = match t[2].parse() {
            Ok(g) => g,
            Err(_) => continue,
        };
        let metric: u32 = match t[4].parse() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if best.map(|(m, _)| metric < m).unwrap_or(true) {
            best = Some((metric, gw));
        }
    }
    best.map(|(_, g)| g)
}

fn is_guid(w: &[u8]) -> bool {
    for (k, &c) in w.iter().enumerate() {
        let dash = matches!(k, 8 | 13 | 18 | 23);
        if dash {
            if c != b'-' {
                return false;
            }
        } else if !c.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

/// Premier GUID (8-4-4-4-12) trouve dans un texte.
pub fn extract_guid(s: &str) -> Option<String> {
    let b = s.as_bytes();
    if b.len() < 36 {
        return None;
    }
    for i in 0..=(b.len() - 36) {
        let w = &b[i..i + 36];
        if is_guid(w) {
            return Some(String::from_utf8_lossy(w).to_string());
        }
    }
    None
}

/// Valeurs hexadecimales `0x...` d'une sortie `powercfg /query`, dans l'ordre d'apparition.
/// Pour un parametre de type liste, les deux dernieres sont l'index secteur (AC) puis batterie (DC).
pub fn parse_powercfg_indices(output: &str) -> Vec<u32> {
    let mut v = Vec::new();
    for line in output.lines() {
        if let Some(i) = line.find("0x") {
            let rest: String = line[i + 2..].chars().take_while(|c| c.is_ascii_hexdigit()).collect();
            if let Ok(n) = u32::from_str_radix(&rest, 16) {
                v.push(n);
            }
        }
    }
    v
}

/// (AC, DC) d'un `powercfg /query` portant sur un seul parametre.
pub fn powercfg_ac_dc(output: &str) -> Option<(u32, u32)> {
    let v = parse_powercfg_indices(output);
    if v.len() < 2 {
        return None;
    }
    Some((v[v.len() - 2], v[v.len() - 1]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROUTE: &str = "\
IPv4 Route Table
===========================================================================
Active Routes:
Network Destination        Netmask          Gateway       Interface  Metric
          0.0.0.0          0.0.0.0      10.0.0.1        10.0.0.5     600
          0.0.0.0          0.0.0.0      192.168.1.1    192.168.1.20     35
        127.0.0.0        255.0.0.0         On-link         127.0.0.1    331
";

    #[test]
    fn gateway_lowest_metric() {
        assert_eq!(parse_route_print(ROUTE), Some(Ipv4Addr::new(192, 168, 1, 1)));
    }

    #[test]
    fn gateway_ignores_on_link_default() {
        let s = "          0.0.0.0          0.0.0.0         On-link     10.8.0.2    10\n";
        assert_eq!(parse_route_print(s), None);
        assert_eq!(parse_route_print(""), None);
    }

    #[test]
    fn guid_extraction() {
        let s = "GUID du modele d'alimentation : 381b4222-f694-41f0-9685-ff5bb260df2e  (Equilibre)";
        assert_eq!(extract_guid(s).as_deref(), Some("381b4222-f694-41f0-9685-ff5bb260df2e"));
        assert_eq!(extract_guid("pas de guid ici"), None);
        assert_eq!(extract_guid("short"), None);
    }

    #[test]
    fn powercfg_values() {
        let s = "\
  GUID du sous-groupe : 19cbb8fa-5279-450e-9fac-8a3d5fedd0c1  (Parametres de carte sans fil)
    GUID du parametre d'alimentation : 12bbebe6-58d6-4636-95bb-3217ef867c1a  (Mode d'economie d'energie)
      Index de parametre possible : 000
      Index de parametre possible : 003
    Index du parametre d'alimentation du courant alternatif actuel : 0x00000003
    Index du parametre d'alimentation du courant continu actuel : 0x00000002
";
        assert_eq!(parse_powercfg_indices(s), vec![3, 2]);
        assert_eq!(powercfg_ac_dc(s), Some((3, 2)));
        assert_eq!(powercfg_ac_dc("rien"), None);
    }
}
