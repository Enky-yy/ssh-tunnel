use crate::config::Config;
use crate::{cf, net, service, ui};
use anyhow::Result;
use serde_json::json;
use std::collections::BTreeSet;

pub async fn run(cfg: &Config, as_json: bool) -> Result<()> {
    let bin = cf::find_binary(Some(&cfg.cloudflared));
    let active = service::is_active(cfg).await;
    let enabled = service::is_enabled(cfg).await;
    let tunnel = match &bin {
        Some(b) => cf::find_tunnel(b, &cfg.tunnel).await.ok().flatten(),
        None => None,
    };
    let addrs = net::resolve(&cfg.hostname, 443).await;
    let ips: Vec<String> = addrs.iter().map(|a| a.ip().to_string()).collect();
    let conns = tunnel.as_ref().map(|t| t.connections.len()).unwrap_or(0);
    let colos: Vec<String> = tunnel
        .as_ref()
        .map(|t| {
            t.connections
                .iter()
                .map(|c| c.colo.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        })
        .unwrap_or_default();

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "tunnel": cfg.tunnel,
                "tunnel_id": cfg.tunnel_id,
                "hostname": cfg.hostname,
                "alias": cfg.alias(),
                "service": { "active": active, "enabled": enabled },
                "connections": conns,
                "colos": colos,
                "dns": ips,
                "ssh_command": format!("ssh {}", cfg.alias()),
            }))?
        );
        return Ok(());
    }

    let state = if active {
        ui::green("active")
    } else {
        ui::red("inactive")
    };
    let boot = if enabled {
        ui::green("enabled")
    } else {
        ui::yellow("disabled")
    };

    println!("{:<12} {} ({})", "tunnel", cfg.tunnel, cfg.tunnel_id);
    println!("{:<12} {}", "hostname", cfg.hostname);
    println!("{:<12} {}", "alias", cfg.alias());
    println!("{:<12} {} / {}", "service", state, boot);
    println!(
        "{:<12} {}{}",
        "connections",
        conns,
        if colos.is_empty() {
            String::new()
        } else {
            format!(" ({})", colos.join(", "))
        }
    );
    println!(
        "{:<12} {}",
        "dns",
        if ips.is_empty() {
            ui::red("does not resolve").to_string()
        } else {
            ips.join(", ")
        }
    );
    println!(
        "{:<12} {}",
        "login",
        ui::bold(&format!("ssh {}", cfg.alias()))
    );
    Ok(())
}
