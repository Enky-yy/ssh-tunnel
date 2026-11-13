use std::net::SocketAddr;
use std::time::Duration;

pub async fn resolve(host: &str, port: u16) -> Vec<SocketAddr> {
    match tokio::net::lookup_host((host, port)).await {
        Ok(iter) => iter.collect(),
        Err(_) => Vec::new(),
    }
}

pub async fn tcp_ok(host: &str, port: u16, secs: u64) -> bool {
    for addr in resolve(host, port).await {
        let attempt = tokio::time::timeout(
            Duration::from_secs(secs),
            tokio::net::TcpStream::connect(addr),
        )
        .await;
        if let Ok(Ok(_)) = attempt {
            return true;
        }
    }
    false
}

/// Split "host:port" (IPv4, IPv6-in-brackets or bare host) into (host, port).
pub fn split_target(target: &str) -> (String, u16) {
    let target = target.trim();
    if let Some(rest) = target.strip_prefix('[')
        && let Some((host, tail)) = rest.split_once(']')
    {
        let port = tail
            .strip_prefix(':')
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(22);
        return (host.to_string(), port);
    }
    match target.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => {
            (host.to_string(), port.parse::<u16>().unwrap_or(22))
        }
        _ => (target.to_string(), 22),
    }
}

#[cfg(test)]
mod tests {
    use super::split_target;

    #[test]
    fn ipv4_target() {
        assert_eq!(split_target("127.0.0.1:22"), ("127.0.0.1".into(), 22));
    }

    #[test]
    fn hostname_target() {
        assert_eq!(split_target("localhost:2222"), ("localhost".into(), 2222));
    }

    #[test]
    fn bare_host_defaults_to_22() {
        assert_eq!(split_target("localhost"), ("localhost".into(), 22));
    }

    #[test]
    fn ipv6_target() {
        assert_eq!(split_target("[::1]:22"), ("::1".into(), 22));
    }
}
