//! Processes that serve something other people use: anything listening on
//! a port that Tailscale Serve or Funnel forwards to. coolant never
//! suggests stopping these.

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

/// Ports Tailscale Serve/Funnel forward to on this machine.
pub fn tailscale_targets() -> BTreeSet<u16> {
    let cli = ["tailscale", "/Applications/Tailscale.app/Contents/MacOS/Tailscale"];
    let Some(out) = cli.iter().find_map(|c| Command::new(c).args(["serve", "status", "--json"]).output().ok()) else {
        return BTreeSet::new();
    };
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else { return BTreeSet::new() };
    let mut ports = BTreeSet::new();
    for site in json.get("Web").and_then(|w| w.as_object()).into_iter().flat_map(|w| w.values()) {
        for h in site.get("Handlers").and_then(|h| h.as_object()).into_iter().flat_map(|h| h.values()) {
            if let Some(port) = h.get("Proxy").and_then(|p| p.as_str()).and_then(port_of) {
                ports.insert(port);
            }
        }
    }
    // Plain TCP forwards: {"TCP": {"443": {"TCPForward": "127.0.0.1:7100"}}}
    for fwd in json.get("TCP").and_then(|t| t.as_object()).into_iter().flat_map(|t| t.values()) {
        if let Some(port) = fwd.get("TCPForward").and_then(|p| p.as_str()).and_then(port_of) {
            ports.insert(port);
        }
    }
    ports
}

fn port_of(url: &str) -> Option<u16> {
    let hostport = url.split("://").last()?.split('/').next()?;
    hostport.rsplit(':').next()?.parse().ok()
}

/// Listening TCP ports of processes we can see, by PID.
pub fn listening() -> BTreeMap<i32, Vec<u16>> {
    let Ok(out) = Command::new("lsof").args(["-nP", "-iTCP", "-sTCP:LISTEN", "-Fpn"]).output() else {
        return BTreeMap::new();
    };
    let mut m: BTreeMap<i32, Vec<u16>> = BTreeMap::new();
    let mut pid = None;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some(p) = line.strip_prefix('p') {
            pid = p.parse().ok();
        } else if let (Some(n), Some(p)) = (line.strip_prefix('n'), pid)
            && let Some(port) = n.rsplit(':').next().and_then(|s| s.parse().ok())
        {
            let ports = m.entry(p).or_default();
            if !ports.contains(&port) {
                ports.push(port);
            }
        }
    }
    m
}

/// PIDs listening on a port Tailscale exposes.
pub fn exposed_pids() -> BTreeSet<i32> {
    let targets = tailscale_targets();
    if targets.is_empty() {
        return BTreeSet::new();
    }
    listening().into_iter().filter(|(_, ports)| ports.iter().any(|p| targets.contains(p))).map(|(pid, _)| pid).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports_from_urls() {
        assert_eq!(port_of("http://127.0.0.1:7100"), Some(7100));
        assert_eq!(port_of("http://localhost:3000/api"), Some(3000));
        assert_eq!(port_of("127.0.0.1:9600"), Some(9600));
    }
}
