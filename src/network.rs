//! HTTP layer, ported from `searx/network/`.
//!
//! Responsibilities kept from upstream: rotating user agents, per request
//! timeout, proxies, a redirect limit, retries on transport errors and a
//! response cache.

use std::collections::{BTreeMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::header::{HeaderMap, HeaderValue};
use sha2::{Digest, Sha256};
use tracing::debug;
use url::{Host, Url};

use crate::cache::{Cache, CachedResponse};
use crate::engine::{EngineRequest, EngineResponse, FailureKind, Method};
use crate::settings::Settings;

const USER_AGENT_OS: &[&str] = &[
    "Windows NT 10.0; Win64; x64",
    "X11; Linux x86_64",
    "Macintosh; Intel Mac OS X 10.15",
    "X11; Ubuntu; Linux x86_64",
];
const USER_AGENT_VERSIONS: &[&str] = &["157.0", "156.0", "155.0", "154.0"];
const USER_AGENT_TEMPLATE: &str =
    "Mozilla/5.0 ({os}; rv:{version}) Gecko/20100101 Firefox/{version}";

#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    #[error("request timed out")]
    Timeout,
    #[error("http status {status}")]
    Status {
        status: u16,
        body: String,
        response: Box<EngineResponse>,
    },
    #[error("transport error: {0}")]
    Transport(String),
    #[error("request could not be built: {0}")]
    Build(String),
}

impl NetworkError {
    /// Map a network error onto the framework failure kinds.
    pub fn failure_kind(&self) -> FailureKind {
        match self {
            Self::Timeout => FailureKind::Timeout,
            Self::Build(_) => FailureKind::Other,
            Self::Transport(_) => FailureKind::Other,
            Self::Status { status, body, .. } => classify_status(*status, body),
        }
    }
}

/// Classify an HTTP status (with a body excerpt) the way upstream does.
pub fn classify_status(status: u16, body: &str) -> FailureKind {
    let lowered = body.to_lowercase();
    let captcha = lowered.contains("captcha")
        || lowered.contains("anomaly")
        || lowered.contains("unusual traffic")
        || lowered.contains("are you a robot");
    match status {
        403 if captcha => FailureKind::Captcha,
        401..=403 => FailureKind::AccessDenied,
        429 => FailureKind::TooManyRequests,
        500..=599 => FailureKind::Http,
        _ => FailureKind::Http,
    }
}

/// Why a URL may not be fetched over the public internet.
///
/// The host string alone is not a proof: `localhost.`, the IPv6 private ranges
/// and internal DNS names all look public to it. [`PublicResolver`] checks the
/// address a name actually resolves to as well.
pub fn check_public_url(url: &Url) -> Result<(), &'static str> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err("only http(s) URLs can be fetched");
    }
    match url.host() {
        Some(Host::Ipv4(address)) => check_public_address(IpAddr::V4(address)),
        Some(Host::Ipv6(address)) => check_public_address(IpAddr::V6(address)),
        Some(Host::Domain(name)) => check_public_name(name),
        None => Err("the URL has no host"),
    }
}

/// Whether a URL may be fetched over the public internet.
pub fn is_public_url(url: &Url) -> bool {
    check_public_url(url).is_ok()
}

fn check_public_address(address: IpAddr) -> Result<(), &'static str> {
    if is_public_ip(address) {
        Ok(())
    } else {
        Err("the host is not a public address")
    }
}

/// DNS suffixes that only exist on a local network.
const LOCAL_SUFFIXES: &[&str] = &[
    "localhost",
    "local",
    "localdomain",
    "internal",
    "intranet",
    "lan",
    "home",
    "home.arpa",
    "corp",
    "private",
    "test",
    "invalid",
];

fn check_public_name(name: &str) -> Result<(), &'static str> {
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    // A single label comes from the local resolver, never from the public DNS.
    if !name.contains('.') {
        return Err("the host is a local name");
    }
    if LOCAL_SUFFIXES
        .iter()
        .any(|suffix| has_label_suffix(&name, suffix))
    {
        return Err("the host is a private name");
    }
    Ok(())
}

/// Whether `name` ends in `suffix` as a whole DNS label.
fn has_label_suffix(name: &str, suffix: &str) -> bool {
    name.strip_suffix(suffix)
        .is_some_and(|head| head.ends_with('.'))
}

/// Whether an address is reachable from the public internet.
pub fn is_public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_public_v4(address),
        IpAddr::V6(address) => is_public_v6(address),
    }
}

fn is_public_v4(address: Ipv4Addr) -> bool {
    let [first, second, third, _] = address.octets();
    !(address.is_unspecified()
        || address.is_loopback()
        || address.is_private()
        || address.is_link_local()
        || address.is_documentation()
        || address.is_multicast()
        // 0.0.0.0/8, 100.64.0.0/10 (CGNAT), 192.0.0.0/24, 198.18.0.0/15 and
        // 240.0.0.0/4 are not routed on the public internet.
        || first == 0
        || (first == 100 && (64..=127).contains(&second))
        || (first == 192 && second == 0 && third == 0)
        || (first == 198 && matches!(second, 18 | 19))
        || first >= 240)
}

fn is_public_v6(address: Ipv6Addr) -> bool {
    if address.is_unspecified() || address.is_loopback() || address.is_multicast() {
        return false;
    }
    // An IPv4-mapped (or the deprecated IPv4-compatible) address reaches the
    // embedded IPv4 address, so it inherits that address's verdict.
    if let Some(address) = address.to_ipv4() {
        return is_public_v4(address);
    }
    let segments = address.segments();
    let unique_local = segments[0] & 0xfe00 == 0xfc00;
    let link_local = segments[0] & 0xffc0 == 0xfe80;
    let documentation = segments[0] == 0x2001 && segments[1] == 0x0db8;
    !(unique_local || link_local || documentation)
}

/// Hosts the operator configured: engine endpoints and proxies may resolve to a
/// private address on purpose, so they stay reachable.
fn trusted_hosts(settings: &Settings) -> std::sync::Arc<HashSet<String>> {
    let mut hosts = HashSet::new();
    let mut add = |url: &str| {
        if let Ok(parsed) = Url::parse(url) {
            if let Some(host) = parsed.host_str() {
                hosts.insert(normalize_host(host));
            }
        }
    };
    for engine in &settings.engines {
        for url in [
            engine.base_url.as_deref(),
            engine.search_url.as_deref(),
            engine.proxy.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            add(url);
        }
        // Engines keep their extra options free-form (`api_url`, `backend_url`…).
        for value in engine.extra.values() {
            if let Some(url) = value.as_str() {
                add(url);
            }
        }
    }
    for proxy in settings.outgoing.proxies.values() {
        add(proxy);
    }
    std::sync::Arc::new(hosts)
}

fn normalize_host(host: &str) -> String {
    host.trim_matches(['[', ']'])
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

/// The system resolver, wrapped so [`PublicResolver`] can check its answers.
struct SystemResolver;

impl Resolve for SystemResolver {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let host = name.as_str().to_string();
            let addresses: Vec<SocketAddr> =
                tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}

/// A resolver that refuses a non-public answer and returns the addresses the
/// connection will use.
///
/// Checking a name and then letting the connector resolve it again would leave
/// a window for a second answer (DNS rebinding). This resolver hands the
/// checked addresses to the connection instead. A configured engine endpoint or
/// proxy may resolve privately.
struct PublicResolver<R> {
    inner: R,
    trusted: std::sync::Arc<HashSet<String>>,
}

impl PublicResolver<SystemResolver> {
    fn system(trusted: std::sync::Arc<HashSet<String>>) -> Self {
        Self::new(SystemResolver, trusted)
    }
}

impl<R> PublicResolver<R> {
    fn new(inner: R, trusted: std::sync::Arc<HashSet<String>>) -> Self {
        Self { inner, trusted }
    }
}

impl<R: Resolve> Resolve for PublicResolver<R> {
    fn resolve(&self, name: Name) -> Resolving {
        let trusted = self.trusted.contains(&normalize_host(name.as_str()));
        let resolved = self.inner.resolve(name);
        Box::pin(async move {
            let addresses: Vec<SocketAddr> = resolved.await?.collect();
            if addresses.is_empty() {
                return Err("the host name resolved to no address".into());
            }
            if !trusted {
                if let Some(address) = addresses.iter().find(|address| !is_public_ip(address.ip()))
                {
                    return Err(format!("{} resolves to a non-public address", address.ip()).into());
                }
            }
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}

/// Follow a redirect only when its target may be fetched from the public
/// internet.
///
/// `Policy::limited` follows a `Location` without looking at it, so a public
/// URL could bounce a proxy onto the local network. A target whose name
/// resolves to a private address is refused by [`PublicResolver`] when the hop
/// connects.
fn redirect_policy(max_redirects: usize) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() > max_redirects {
            return attempt.error("too many redirects");
        }
        match check_public_url(attempt.url()) {
            Ok(()) => attempt.follow(),
            Err(reason) => attempt.error(format!("redirect refused: {reason}")),
        }
    })
}

/// The HTTP client used by every engine.
pub struct Network {
    client: reqwest::Client,
    /// Clients for engines that need their own proxy, keyed by proxy URL.
    proxied: std::sync::Mutex<std::collections::HashMap<String, reqwest::Client>>,
    /// Settings kept so a proxied client can be rebuilt on demand.
    outgoing: crate::settings::Outgoing,
    user_agents: Vec<String>,
    suffix: String,
    retries: u32,
    counter: AtomicUsize,
    /// Operator configured hosts that may resolve to a private address.
    trusted_hosts: std::sync::Arc<HashSet<String>>,
}

/// Build a client from the outgoing settings, optionally through another proxy.
fn build_client(
    outgoing: &crate::settings::Outgoing,
    extra_proxy: Option<reqwest::Proxy>,
    trusted: std::sync::Arc<HashSet<String>>,
) -> Result<reqwest::Client, NetworkError> {
    let mut builder = reqwest::Client::builder()
        .redirect(redirect_policy(outgoing.max_redirects))
        .dns_resolver(std::sync::Arc::new(PublicResolver::system(trusted)))
        .timeout(Duration::from_secs_f64(
            outgoing.max_request_timeout.max(1.0),
        ))
        .connect_timeout(Duration::from_secs_f64(
            (outgoing.request_timeout / 2.0).clamp(1.0, 5.0),
        ))
        .pool_max_idle_per_host(outgoing.pool_maxsize)
        .danger_accept_invalid_certs(!outgoing.verify)
        .user_agent(default_user_agent());
    if let Some(path) = &outgoing.ca_bundle {
        let pem = std::fs::read(path)
            .map_err(|e| NetworkError::Build(format!("cannot read CA bundle {path}: {e}")))?;
        let certificates = reqwest::Certificate::from_pem_bundle(&pem)
            .map_err(|e| NetworkError::Build(format!("invalid CA bundle: {e}")))?;
        if certificates.is_empty() {
            return Err(NetworkError::Build(
                "CA bundle contains no certificates".into(),
            ));
        }
        for certificate in certificates {
            builder = builder.add_root_certificate(certificate);
        }
    }
    if !outgoing.enable_http2 {
        builder = builder.http1_only();
    }
    for (scheme, proxy) in &outgoing.proxies {
        // One proxy per scheme (upstream's `outgoing.proxies`); the scheme key
        // selects which requests it applies to in SearXNG, reqwest proxies all.
        let proxy =
            reqwest::Proxy::all(proxy).map_err(|error| NetworkError::Build(error.to_string()))?;
        tracing::debug!(scheme, "configured proxy");
        builder = builder.proxy(proxy);
    }
    if let Some(proxy) = extra_proxy {
        builder = builder.proxy(proxy);
    }
    builder
        .build()
        .map_err(|error| NetworkError::Build(error.to_string()))
}
impl Network {
    pub fn from_settings(settings: &Settings) -> Result<Self, NetworkError> {
        let outgoing = &settings.outgoing;
        let trusted = trusted_hosts(settings);
        let client = build_client(outgoing, None, std::sync::Arc::clone(&trusted))?;
        Ok(Self {
            client,
            proxied: std::sync::Mutex::new(std::collections::HashMap::new()),
            outgoing: outgoing.clone(),
            user_agents: build_user_agents(),
            suffix: outgoing.useragent_suffix.clone(),
            retries: outgoing.retries,
            counter: AtomicUsize::new(0),
            trusted_hosts: trusted,
        })
    }

    /// The client for one request, honouring a per-engine proxy.
    ///
    /// Engines share the default client; a per-engine proxy gets its own so a
    /// blocked engine can be routed elsewhere without moving the rest.
    fn client_for(&self, proxy: Option<&str>) -> reqwest::Client {
        let Some(proxy) = proxy.map(str::trim).filter(|value| !value.is_empty()) else {
            return self.client.clone();
        };
        tracing::debug!(proxy, "using a per-engine proxy");
        if let Ok(cache) = self.proxied.lock() {
            if let Some(client) = cache.get(proxy) {
                return client.clone();
            }
        }
        let Ok(configured) = reqwest::Proxy::all(proxy) else {
            return self.client.clone();
        };
        let Ok(client) = build_client(
            &self.outgoing,
            Some(configured),
            std::sync::Arc::clone(&self.trusted_hosts),
        ) else {
            return self.client.clone();
        };
        if let Ok(mut cache) = self.proxied.lock() {
            cache.insert(proxy.to_string(), client.clone());
        }
        client
    }

    /// A different user agent on every call, like upstream's rotation.
    pub fn user_agent(&self) -> String {
        if self.user_agents.is_empty() {
            return default_user_agent();
        }
        let index = self.counter.fetch_add(1, Ordering::Relaxed) % self.user_agents.len();
        format!("{} {}", self.user_agents[index], self.suffix)
            .trim()
            .to_string()
    }

    fn default_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            reqwest::header::ACCEPT,
            HeaderValue::from_static(
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            ),
        );
        headers.insert(
            reqwest::header::ACCEPT_LANGUAGE,
            HeaderValue::from_static("en-US,en;q=0.7"),
        );
        headers.insert(
            reqwest::header::HeaderName::from_static("dnt"),
            HeaderValue::from_static("1"),
        );
        if let Ok(value) = HeaderValue::from_str(&self.user_agent()) {
            headers.insert(reqwest::header::USER_AGENT, value);
        }
        headers
    }

    /// Fetch a request, using the cache when one is given.
    pub async fn fetch(
        &self,
        request: EngineRequest,
        timeout: Duration,
        cache: Option<&Cache>,
        proxy: Option<&str>,
    ) -> Result<EngineResponse, NetworkError> {
        let deadline = tokio::time::Instant::now() + timeout;
        let client = self.client_for(proxy);
        let mut headers: Vec<_> = request
            .headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_bytes()))
            .collect();
        headers.sort_by_key(|(name, _)| *name);
        let identity = serde_json::to_vec(&(
            request.method,
            &request.url,
            headers,
            &request.cookies,
            &request.form,
            &request.json,
            proxy.map(str::trim),
        ))
        .map_err(|error| NetworkError::Build(error.to_string()))?;
        let cache_key = format!("{:x}", Sha256::digest(identity));
        if let Some(cache) = cache {
            if let Some(cached) = tokio::time::timeout_at(deadline, cache.get(&cache_key))
                .await
                .map_err(|_| NetworkError::Timeout)?
            {
                debug!(url = %request.url, "cache hit");
                return Ok(EngineResponse {
                    status: cached.status,
                    final_url: cached.final_url,
                    headers: cached.headers,
                    body: cached.body.as_ref().clone(),
                });
            }
        }

        let mut attempt = 0;
        loop {
            attempt += 1;
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(NetworkError::Timeout);
            }
            let outcome = self.send(client.clone(), &request, remaining).await;
            match outcome {
                Ok(response) => {
                    if let Some(cache) = cache {
                        if response.status == 200
                            && !response.body.is_empty()
                            && tokio::time::timeout_at(
                                deadline,
                                cache.set(
                                    &cache_key,
                                    CachedResponse {
                                        status: response.status,
                                        final_url: response.final_url.clone(),
                                        headers: response.headers.clone(),
                                        body: std::sync::Arc::new(response.body.clone()),
                                    },
                                ),
                            )
                            .await
                            .is_err()
                        {
                            debug!(url = %request.url, "cache write timed out");
                        }
                    }
                    // A 429 usually clears within a second. Retrying once keeps
                    // a good engine (Yahoo! JAPAN, Bing) alive instead of
                    // suspending it for minutes.
                    if response.status == 429 && attempt <= self.retries {
                        let backoff = Duration::from_millis(700);
                        if tokio::time::Instant::now() + backoff < deadline {
                            debug!(url = %request.url, "rate limited, retrying after a pause");
                            tokio::time::sleep(backoff).await;
                            continue;
                        }
                    }
                    if response.status >= 400 {
                        return Err(NetworkError::Status {
                            status: response.status,
                            body: String::from_utf8_lossy(
                                &response.body[..response.body.len().min(4096)],
                            )
                            .to_string(),
                            response: Box::new(response),
                        });
                    }
                    return Ok(response);
                }
                Err(error) if attempt <= self.retries && is_retryable(&error) => {
                    // never retry a timeout: the engine deadline already elapsed
                    if matches!(error, NetworkError::Timeout) {
                        return Err(error);
                    }
                    let backoff = Duration::from_millis(250 * attempt as u64);
                    if tokio::time::Instant::now() + backoff >= deadline {
                        return Err(error);
                    }
                    debug!(url = %request.url, %error, "retrying request");
                    tokio::time::sleep(backoff).await;
                }
                Err(error) => return Err(error),
            }
        }
    }

    async fn send(
        &self,
        client: reqwest::Client,
        request: &EngineRequest,
        timeout: Duration,
    ) -> Result<EngineResponse, NetworkError> {
        let method = match request.method {
            Method::Get => reqwest::Method::GET,
            Method::Post => reqwest::Method::POST,
        };
        let mut headers = self.default_headers();
        for (name, value) in request.headers.iter() {
            headers.insert(name.clone(), value.clone());
        }
        if !request.cookies.is_empty() {
            let cookie = request
                .cookies
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join("; ");
            if let Ok(value) = HeaderValue::from_str(&cookie) {
                headers.insert(reqwest::header::COOKIE, value);
            }
        }

        let mut builder = client.request(method, &request.url).headers(headers);
        if let Some(json) = &request.json {
            builder = builder.json(json);
        } else if request.method == Method::Post {
            builder = builder.form(&request.form);
        }

        let send = async {
            let response = builder.send().await.map_err(|error| {
                if error.is_timeout() {
                    NetworkError::Timeout
                } else {
                    NetworkError::Transport(error.to_string())
                }
            })?;
            let status = response.status().as_u16();
            let final_url = response.url().to_string();
            let mut headers = BTreeMap::new();
            for (name, value) in response.headers().iter() {
                if let Ok(value) = value.to_str() {
                    headers.insert(name.as_str().to_lowercase(), value.to_string());
                }
            }
            let body = response
                .bytes()
                .await
                .map_err(|error| NetworkError::Transport(error.to_string()))?
                .to_vec();
            Ok(EngineResponse {
                status,
                final_url,
                headers,
                body,
            })
        };

        match tokio::time::timeout(timeout, send).await {
            Ok(result) => result,
            Err(_) => Err(NetworkError::Timeout),
        }
    }
}

fn is_retryable(error: &NetworkError) -> bool {
    match error {
        NetworkError::Transport(_) => true,
        // a timeout already consumed the whole engine budget
        NetworkError::Timeout => false,
        NetworkError::Status { status, .. } => *status == 429 || *status >= 500,
        NetworkError::Build(_) => false,
    }
}

/// The default user agent, mirroring upstream's `searxng_useragent`.
pub fn default_user_agent() -> String {
    build_user_agents()
        .into_iter()
        .next()
        .unwrap_or_else(|| "rustxng".to_string())
}

/// Build the user agent pool like upstream's `data/useragents.json`.
fn build_user_agents() -> Vec<String> {
    let mut agents = Vec::new();
    for os in USER_AGENT_OS {
        for version in USER_AGENT_VERSIONS {
            agents.push(
                USER_AGENT_TEMPLATE
                    .replace("{os}", os)
                    .replace("{version}", version),
            );
        }
    }
    agents
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn network_with(client: reqwest::Client) -> Network {
        Network {
            client,
            proxied: std::sync::Mutex::new(std::collections::HashMap::new()),
            outgoing: crate::settings::Outgoing::default(),
            user_agents: build_user_agents(),
            suffix: String::new(),
            retries: 0,
            counter: AtomicUsize::new(0),
            trusted_hosts: std::sync::Arc::new(HashSet::new()),
        }
    }

    #[test]
    fn user_agents_are_diverse_and_plausible() {
        let network = network_with(reqwest::Client::new());
        let first = network.user_agent();
        let second = network.user_agent();
        assert!(first.starts_with("Mozilla/5.0"));
        assert_ne!(first, second);
    }

    #[test]
    fn captcha_pages_are_recognised() {
        assert_eq!(
            classify_status(403, "please solve the captcha"),
            FailureKind::Captcha
        );
        assert_eq!(classify_status(403, "denied"), FailureKind::AccessDenied);
        assert_eq!(classify_status(429, ""), FailureKind::TooManyRequests);
        assert_eq!(classify_status(500, ""), FailureKind::Http);
    }

    /// Answer every connection with `response` and count the connections.
    async fn stub_server(response: String) -> (SocketAddr, Arc<AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind the stub");
        let address = listener.local_addr().expect("stub address");
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                counter.fetch_add(1, Ordering::SeqCst);
                let response = response.clone();
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut chunk = [0u8; 1024];
                    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                        match stream.read(&mut chunk).await {
                            Ok(0) | Err(_) => break,
                            Ok(read) => request.extend_from_slice(&chunk[..read]),
                        }
                    }
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        (address, hits)
    }

    fn redirect_response(location: &str) -> String {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
    }

    /// Answer DNS with a fixed address list, so a test never needs the network.
    struct StubResolver {
        addresses: Vec<SocketAddr>,
    }

    impl Resolve for StubResolver {
        fn resolve(&self, _name: Name) -> Resolving {
            let addresses = self.addresses.clone();
            Box::pin(async move { Ok(Box::new(addresses.into_iter()) as Addrs) })
        }
    }

    #[tokio::test]
    async fn a_redirect_to_a_private_address_is_refused_before_connecting() {
        let (target, target_hits) = stub_server(String::new()).await;
        let (first, _) = stub_server(redirect_response(&format!(
            "http://127.0.0.1:{}/secret",
            target.port()
        )))
        .await;
        let client = reqwest::Client::builder()
            .redirect(redirect_policy(5))
            .resolve("stub.test", first)
            .build()
            .expect("test client");
        let error = network_with(client)
            .fetch(
                EngineRequest::get(format!("http://stub.test:{}/", first.port())),
                Duration::from_secs(5),
                None,
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(error, NetworkError::Transport(_)));
        assert_eq!(
            target_hits.load(Ordering::SeqCst),
            0,
            "the private address must not be contacted"
        );
    }

    #[tokio::test]
    async fn a_redirect_to_a_name_resolving_to_a_private_address_is_refused_before_connecting() {
        let (target, target_hits) = stub_server(String::new()).await;
        let (first, _) = stub_server(redirect_response(&format!(
            "http://internal.example.com:{}/secret",
            target.port()
        )))
        .await;
        let resolver = Arc::new(PublicResolver::new(
            StubResolver {
                addresses: vec![SocketAddr::from(([127, 0, 0, 1], 0))],
            },
            Arc::new(HashSet::new()),
        ));
        let client = reqwest::Client::builder()
            .redirect(redirect_policy(5))
            .dns_resolver(resolver)
            .resolve("stub.test", first)
            .build()
            .expect("test client");
        let error = network_with(client)
            .fetch(
                EngineRequest::get(format!("http://stub.test:{}/", first.port())),
                Duration::from_secs(5),
                None,
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(error, NetworkError::Transport(_)));
        assert_eq!(
            target_hits.load(Ordering::SeqCst),
            0,
            "the address behind the name must not be contacted"
        );
    }

    #[tokio::test]
    async fn a_redirect_to_another_scheme_is_refused() {
        // `file:` URLs have no authority, so the redirect layer drops them before
        // any policy sees them; a scheme with an authority reaches the policy.
        let (first, _) = stub_server(redirect_response("ftp://example.com/passwd")).await;
        let client = reqwest::Client::builder()
            .redirect(redirect_policy(5))
            .resolve("stub.test", first)
            .build()
            .expect("test client");
        let error = network_with(client)
            .fetch(
                EngineRequest::get(format!("http://stub.test:{}/", first.port())),
                Duration::from_secs(5),
                None,
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(error, NetworkError::Transport(_)));
    }

    #[tokio::test]
    async fn a_configured_host_may_resolve_to_a_private_address() {
        let resolver = PublicResolver::new(
            StubResolver {
                addresses: vec![SocketAddr::from(([10, 0, 0, 5], 7700))],
            },
            Arc::new(HashSet::from(["meilisearch".to_string()])),
        );
        let name: Name = "meilisearch".parse().expect("name");
        let addresses: Vec<SocketAddr> = resolver.resolve(name).await.expect("trusted").collect();
        assert_eq!(addresses, vec![SocketAddr::from(([10, 0, 0, 5], 7700))]);
    }

    #[test]
    fn engine_endpoints_and_proxies_are_trusted_hosts() {
        let mut settings = Settings::defaults();
        let mut engine =
            crate::settings::EngineConfig::new("meilisearch", "meilisearch", "mes", &["general"]);
        engine.base_url = Some("http://meilisearch:7700".to_string());
        engine.proxy = Some("socks5h://corp-proxy:1080".to_string());
        engine.extra.insert(
            "api_url".to_string(),
            toml::Value::String("http://intranet-api:9000".to_string()),
        );
        settings.engines = vec![engine];
        settings
            .outgoing
            .proxies
            .insert("https".to_string(), "http://egress:3128".to_string());

        let hosts = trusted_hosts(&settings);
        assert!(hosts.contains("meilisearch"));
        assert!(hosts.contains("corp-proxy"));
        assert!(hosts.contains("intranet-api"));
        assert!(hosts.contains("egress"));
    }
}
