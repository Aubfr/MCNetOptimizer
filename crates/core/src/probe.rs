//! Sondes reseau actives, a tres faible cadence et sans charge.

use std::net::{Shutdown, SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

/// Temps d'etablissement d'une connexion TCP (SYN -> SYN/ACK) en ms, None si echec/timeout.
/// La connexion est fermee immediatement, aucune donnee n'est envoyee.
pub fn tcp_connect_rtt(addr: SocketAddr, timeout: Duration) -> Option<f32> {
    let start = Instant::now();
    match TcpStream::connect_timeout(&addr, timeout) {
        Ok(stream) => {
            let ms = start.elapsed().as_secs_f32() * 1000.0;
            let _ = stream.shutdown(Shutdown::Both);
            Some(ms)
        }
        Err(_) => None,
    }
}

/// Resout un nom en adresse IPv4 (Minecraft 1.8.9 / Java prefere IPv4).
pub fn resolve_ipv4(host: &str, port: u16) -> Option<SocketAddr> {
    (host, port).to_socket_addrs().ok()?.find(|a| a.is_ipv4())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn tcp_probe_on_local_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let rtt = tcp_connect_rtt(addr, Duration::from_millis(1000));
        assert!(rtt.is_some());
        assert!(rtt.unwrap() < 1000.0);
    }

    #[test]
    fn tcp_probe_closed_port_fails() {
        // Port ephemere ouvert puis ferme : la connexion doit echouer.
        let addr = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap()
        };
        assert!(tcp_connect_rtt(addr, Duration::from_millis(500)).is_none());
    }

    #[test]
    fn resolve_literal_ip() {
        let a = resolve_ipv4("127.0.0.1", 25565).unwrap();
        assert_eq!(a.port(), 25565);
        assert!(a.is_ipv4());
    }
}
