// SPDX-License-Identifier: AGPL-3.0-or-later
//! Streamable HTTP transport for the MCP server (`rustxng mcp --http`).
//!
//! stdio starts one server per process. Streamable HTTP starts one
//! [`McpServer`] per MCP session, so `read_page` keeps its URL allowlist and
//! page cache inside that session.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};

use super::McpServer;
use crate::aggregator::Searcher;

/// Serve MCP over Streamable HTTP at `/mcp` until the process stops.
pub async fn serve(searcher: Searcher, address: SocketAddr) -> Result<(), String> {
    let allowed_hosts = std::env::var("RUSTXNG_MCP_ALLOWED_HOSTS").ok();
    let config = config_for(address, allowed_hosts.as_deref())?;
    let app = router(Arc::new(searcher), Vec::new(), config);
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|error| format!("could not listen on {address}: {error}"))?;
    let bound = listener.local_addr().unwrap_or(address);
    tracing::info!("MCP Streamable HTTP listening on http://{bound}/mcp");
    axum::serve(listener, app)
        .await
        .map_err(|error| format!("MCP HTTP server failed: {error}"))
}

/// Build the MCP router. Tests pin public hostnames to loopback pages.
pub fn router(
    searcher: Arc<Searcher>,
    page_overrides: Vec<(String, SocketAddr)>,
    config: StreamableHttpServerConfig,
) -> Router {
    let service = StreamableHttpService::new(
        move || {
            McpServer::with_page_overrides(Arc::clone(&searcher), &page_overrides)
                .map_err(std::io::Error::other)
        },
        LocalSessionManager::default().into(),
        config,
    );
    Router::new().nest_service("/mcp", service)
}

/// Loopback needs no host list. A concrete non-loopback address allows its own
/// IP. A wildcard bind must name its public hosts: DNS rebinding protection
/// cannot guess them.
fn config_for(
    address: SocketAddr,
    allowed_hosts: Option<&str>,
) -> Result<StreamableHttpServerConfig, String> {
    let config = StreamableHttpServerConfig::default();
    if address.ip().is_loopback() && allowed_hosts.is_none() {
        return Ok(config);
    }
    if let Some(value) = allowed_hosts {
        let hosts: Vec<String> = value
            .split(',')
            .map(str::trim)
            .filter(|host| !host.is_empty())
            .map(str::to_string)
            .collect();
        if hosts.is_empty() {
            return Err("RUSTXNG_MCP_ALLOWED_HOSTS must name at least one host".into());
        }
        return Ok(config.with_allowed_hosts(hosts));
    }
    if address.ip().is_unspecified() {
        return Err(
            "serving MCP HTTP on a wildcard address needs RUSTXNG_MCP_ALLOWED_HOSTS".into(),
        );
    }
    let mut hosts = config.allowed_hosts.clone();
    hosts.push(address.ip().to_string());
    Ok(config.with_allowed_hosts(hosts))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_keeps_the_default_hosts() {
        let config = config_for("127.0.0.1:29483".parse().unwrap(), None).unwrap();
        assert_eq!(config.allowed_hosts, ["localhost", "127.0.0.1", "::1"]);
    }

    #[test]
    fn wildcard_binds_require_explicit_hosts() {
        let address = "0.0.0.0:29483".parse().unwrap();
        assert!(config_for(address, None).is_err());
        assert!(config_for(address, Some(" , ")).is_err());
        let config = config_for(address, Some("search.example, search.example:8443")).unwrap();
        assert_eq!(
            config.allowed_hosts,
            ["search.example", "search.example:8443"]
        );
    }

    #[test]
    fn a_concrete_non_loopback_address_allows_its_ip() {
        let config = config_for("192.0.2.10:29483".parse().unwrap(), None).unwrap();
        assert!(config.allowed_hosts.iter().any(|host| host == "192.0.2.10"));
    }
}
