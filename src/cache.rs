//! In-memory HTTP response cache, ported from `searx/cache.py`.
//!
//! Upstream configuration equivalents are `cache.ttl` (default 300 s) and
//! `cache.max_bytes` (default 8 MB); the cache is a bounded LRU with a TTL.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Cached response payload (a clone of the parts engines look at).
#[derive(Debug, Clone)]
pub struct CachedResponse {
    pub status: u16,
    pub final_url: String,
    pub headers: std::collections::BTreeMap<String, String>,
    pub body: std::sync::Arc<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStats {
    pub hit: u64,
    pub miss: u64,
    pub entries: usize,
    pub bytes: usize,
    pub error: u64,
}

struct Entry {
    stored: Instant,
    response: CachedResponse,
    size: usize,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<String, Entry>,
    order: VecDeque<String>,
    bytes: usize,
}

/// A TTL + LRU cache for engine responses.
pub struct ResponseCache {
    inner: Mutex<Inner>,
    ttl: Duration,
    max_bytes: usize,
    hit: AtomicU64,
    miss: AtomicU64,
    error: AtomicU64,
}

impl Default for ResponseCache {
    fn default() -> Self {
        Self::new(Duration::from_secs(300), 8 * 1024 * 1024)
    }
}

impl ResponseCache {
    pub fn new(ttl: Duration, max_bytes: usize) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            ttl,
            max_bytes,
            hit: AtomicU64::new(0),
            miss: AtomicU64::new(0),
            error: AtomicU64::new(0),
        }
    }

    /// The cache key of a request: method + URL + body.
    pub fn key(method: &str, url: &str, body: &str) -> String {
        format!("{method} {url} {body}")
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn get(&self, key: &str) -> Option<CachedResponse> {
        let mut inner = self.lock();
        let expired = inner
            .entries
            .get(key)
            .map(|entry| entry.stored.elapsed() > self.ttl)
            .unwrap_or(false);
        if expired {
            self.remove_locked(&mut inner, key);
            self.miss.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        if let Some(entry) = inner.entries.get(key) {
            let response = entry.response.clone();
            self.hit.fetch_add(1, Ordering::Relaxed);
            Some(response)
        } else {
            self.miss.fetch_add(1, Ordering::Relaxed);
            None
        }
    }

    pub fn set(&self, key: &str, response: CachedResponse) {
        let size = response.body.len() + key.len() + 128;
        if size > self.max_bytes {
            self.error.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let mut inner = self.lock();
        self.remove_locked(&mut inner, key);
        inner.bytes += size;
        inner.entries.insert(
            key.to_string(),
            Entry {
                stored: Instant::now(),
                response,
                size,
            },
        );
        inner.order.push_back(key.to_string());
        self.evict_locked(&mut inner);
    }

    fn remove_locked(&self, inner: &mut Inner, key: &str) {
        if let Some(entry) = inner.entries.remove(key) {
            inner.bytes = inner.bytes.saturating_sub(entry.size);
            if let Some(position) = inner.order.iter().position(|candidate| candidate == key) {
                inner.order.remove(position);
            }
        }
    }

    fn evict_locked(&self, inner: &mut Inner) {
        while inner.bytes > self.max_bytes {
            let Some(key) = inner.order.pop_front() else {
                break;
            };
            if let Some(entry) = inner.entries.remove(&key) {
                inner.bytes = inner.bytes.saturating_sub(entry.size);
            }
        }
    }

    /// Drop every entry (used by tests and `/stats`).
    pub fn clear(&self) {
        let mut inner = self.lock();
        inner.entries.clear();
        inner.order.clear();
        inner.bytes = 0;
    }

    pub fn stats(&self) -> CacheStats {
        let inner = self.lock();
        CacheStats {
            hit: self.hit.load(Ordering::Relaxed),
            miss: self.miss.load(Ordering::Relaxed),
            entries: inner.entries.len(),
            bytes: inner.bytes,
            error: self.error.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(body: &str) -> CachedResponse {
        CachedResponse {
            status: 200,
            final_url: "https://example.com/".into(),
            headers: Default::default(),
            body: std::sync::Arc::new(body.as_bytes().to_vec()),
        }
    }

    #[test]
    fn stored_responses_are_returned() {
        let cache = ResponseCache::default();
        cache.set("GET https://example.com/ ", response("hello"));
        let cached = cache.get("GET https://example.com/ ").expect("cached");
        assert_eq!(cached.body.len(), 5);
        assert_eq!(cache.stats().hit, 1);
    }

    #[test]
    fn missing_keys_count_as_misses() {
        let cache = ResponseCache::default();
        assert!(cache.get("nope").is_none());
        assert_eq!(cache.stats().miss, 1);
    }

    #[test]
    fn entries_expire_after_the_ttl() {
        let cache = ResponseCache::new(Duration::from_millis(0), 1024);
        cache.set("key", response("x"));
        std::thread::sleep(Duration::from_millis(2));
        assert!(cache.get("key").is_none());
    }

    #[test]
    fn the_cache_stays_within_its_size_budget() {
        let cache = ResponseCache::new(Duration::from_secs(60), 200);
        for index in 0..50 {
            cache.set(&format!("key-{index}"), response(&"x".repeat(64)));
        }
        assert!(cache.stats().bytes <= 200);
    }

    #[test]
    fn oversized_responses_are_not_cached() {
        let cache = ResponseCache::new(Duration::from_secs(60), 16);
        cache.set("key", response(&"x".repeat(100)));
        assert!(cache.get("key").is_none());
    }
}

/// A cache backend: in-process memory or an external Valkey / Redis server.
pub enum CacheBackend {
    Memory(ResponseCache),
    Valkey(ValkeyCache),
}

/// Cache facade used by the network layer and the HTTP handlers.
pub struct Cache {
    backend: CacheBackend,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            backend: CacheBackend::Memory(ResponseCache::default()),
        }
    }
}

impl Cache {
    pub fn memory(ttl: Duration, max_bytes: usize) -> Self {
        Self {
            backend: CacheBackend::Memory(ResponseCache::new(ttl, max_bytes)),
        }
    }

    /// A Valkey / Redis backed cache (upstream valkey.url). The connection is
    /// opened on first use so the searcher can be built synchronously.
    pub fn valkey(url: &str, ttl: Duration) -> Self {
        Self {
            backend: CacheBackend::Valkey(ValkeyCache::new(url, ttl)),
        }
    }

    pub fn is_valkey(&self) -> bool {
        matches!(self.backend, CacheBackend::Valkey(_))
    }

    pub fn key(method: &str, url: &str, body: &str) -> String {
        ResponseCache::key(method, url, body)
    }

    pub async fn get(&self, key: &str) -> Option<CachedResponse> {
        match &self.backend {
            CacheBackend::Memory(cache) => cache.get(key),
            CacheBackend::Valkey(cache) => cache.get(key).await,
        }
    }

    pub async fn set(&self, key: &str, response: CachedResponse) {
        match &self.backend {
            CacheBackend::Memory(cache) => cache.set(key, response),
            CacheBackend::Valkey(cache) => cache.set(key, response).await,
        }
    }

    pub async fn clear(&self) {
        match &self.backend {
            CacheBackend::Memory(cache) => cache.clear(),
            CacheBackend::Valkey(cache) => cache.clear().await,
        }
    }

    pub fn stats(&self) -> CacheStats {
        match &self.backend {
            CacheBackend::Memory(cache) => cache.stats(),
            CacheBackend::Valkey(cache) => cache.stats(),
        }
    }
}

/// A cache stored in Valkey / Redis, like the upstream valkey cache backend.
pub struct ValkeyCache {
    url: String,
    connection: tokio::sync::Mutex<Option<ValkeyConnection>>,
    ttl: Duration,
    hit: AtomicU64,
    miss: AtomicU64,
    error: AtomicU64,
    entries: AtomicU64,
}

impl ValkeyCache {
    pub fn new(url: &str, ttl: Duration) -> Self {
        Self {
            url: url.to_string(),
            connection: tokio::sync::Mutex::new(None),
            ttl,
            hit: AtomicU64::new(0),
            miss: AtomicU64::new(0),
            error: AtomicU64::new(0),
            entries: AtomicU64::new(0),
        }
    }

    async fn command(&self, args: &[&str]) -> Result<Option<String>, String> {
        let mut pooled = self.connection.lock().await;
        // Cancellation must discard the connection.
        let mut connection = match pooled.take() {
            Some(connection) => connection,
            None => ValkeyConnection::connect(&self.url).await?,
        };
        let response = connection.command(args).await;
        if response.is_ok() {
            *pooled = Some(connection);
        }
        response
    }

    pub async fn get(&self, key: &str) -> Option<CachedResponse> {
        match self.command(&["GET", key]).await {
            Ok(Some(payload)) => match decode_response(&payload) {
                Some(response) => {
                    self.hit.fetch_add(1, Ordering::Relaxed);
                    Some(response)
                }
                None => {
                    self.error.fetch_add(1, Ordering::Relaxed);
                    self.miss.fetch_add(1, Ordering::Relaxed);
                    None
                }
            },
            Ok(None) => {
                self.miss.fetch_add(1, Ordering::Relaxed);
                None
            }
            Err(_) => {
                self.error.fetch_add(1, Ordering::Relaxed);
                self.miss.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }

    pub async fn set(&self, key: &str, response: CachedResponse) {
        let Some(payload) = encode_response(&response) else {
            self.error.fetch_add(1, Ordering::Relaxed);
            return;
        };
        let ttl = self.ttl.as_secs().max(1).to_string();
        match self.command(&["SET", key, &payload, "EX", &ttl]).await {
            Ok(_) => {
                self.entries.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => {
                self.error.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub async fn clear(&self) {
        if let Ok(Some(keys)) = self.command(&["KEYS", "searxng:*"]).await {
            for key in keys.split('\n') {
                if !key.is_empty() {
                    let _ = self.command(&["DEL", key]).await;
                }
            }
        }
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hit: self.hit.load(Ordering::Relaxed),
            miss: self.miss.load(Ordering::Relaxed),
            entries: self.entries.load(Ordering::Relaxed) as usize,
            bytes: 0,
            error: self.error.load(Ordering::Relaxed),
        }
    }
}

/// Encode a cached response as JSON with a base64 body.
fn encode_response(response: &CachedResponse) -> Option<String> {
    use base64::Engine as _;
    let document = serde_json::json!({
        "status": response.status,
        "final_url": response.final_url,
        "headers": response.headers,
        "body": base64::engine::general_purpose::STANDARD.encode(response.body.as_slice()),
    });
    serde_json::to_string(&document).ok()
}

/// Decode the payload written by encode_response.
fn decode_response(payload: &str) -> Option<CachedResponse> {
    use base64::Engine as _;
    let document: serde_json::Value = serde_json::from_str(payload).ok()?;
    let body = document.get("body").and_then(|value| value.as_str())?;
    let body = base64::engine::general_purpose::STANDARD
        .decode(body)
        .ok()?;
    let headers = document
        .get("headers")
        .and_then(|value| value.as_object())
        .map(|object| {
            object
                .iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    Some(CachedResponse {
        status: document.get("status").and_then(|value| value.as_u64())? as u16,
        final_url: document
            .get("final_url")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string(),
        headers,
        body: std::sync::Arc::new(body),
    })
}

/// A tiny RESP client over a Tokio stream.
struct ValkeyConnection {
    reader: tokio::io::BufReader<tokio::net::tcp::OwnedReadHalf>,
    writer: tokio::net::tcp::OwnedWriteHalf,
}

impl ValkeyConnection {
    async fn connect(url: &str) -> Result<Self, String> {
        let rest = url
            .strip_prefix("redis://")
            .or_else(|| url.strip_prefix("valkey://"))
            .ok_or_else(|| format!("unsupported cache url: {url}"))?;
        let (authority, database) = match rest.split_once('/') {
            Some((authority, database)) => (authority, database.parse::<i64>().unwrap_or(0)),
            None => (rest, 0),
        };
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => (host.to_string(), port.parse::<u16>().unwrap_or(6379)),
            None => (authority.to_string(), 6379),
        };
        let stream = tokio::net::TcpStream::connect((host.as_str(), port))
            .await
            .map_err(|error| format!("cache: cannot connect to {host}:{port}: {error}"))?;
        let (reader, writer) = stream.into_split();
        let mut connection = Self {
            reader: tokio::io::BufReader::new(reader),
            writer,
        };
        if database != 0 {
            let database = database.to_string();
            connection.command(&["SELECT", &database]).await?;
        }
        Ok(connection)
    }

    /// Run one command; bulk replies come back as Some(String), nil as None.
    async fn command(&mut self, args: &[&str]) -> Result<Option<String>, String> {
        use tokio::io::AsyncWriteExt as _;
        let mut request = format!("*{}\r\n", args.len());
        for argument in args {
            request.push_str(&format!("${}\r\n{}\r\n", argument.len(), argument));
        }
        self.writer
            .write_all(request.as_bytes())
            .await
            .map_err(|error| error.to_string())?;
        self.writer
            .flush()
            .await
            .map_err(|error| error.to_string())?;
        match self.read_reply().await? {
            ValkeyReply::Bulk(value) => Ok(value),
            ValkeyReply::Simple(value) => Ok(Some(value)),
            ValkeyReply::Integer(value) => Ok(Some(value.to_string())),
            ValkeyReply::Nil => Ok(None),
            ValkeyReply::Array(items) => Ok(Some(
                items
                    .into_iter()
                    .filter_map(|item| match item {
                        ValkeyReply::Bulk(Some(value)) | ValkeyReply::Simple(value) => Some(value),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            )),
        }
    }

    fn read_reply(
        &mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ValkeyReply, String>> + Send + '_>>
    {
        Box::pin(self.read_reply_inner())
    }

    async fn read_reply_inner(&mut self) -> Result<ValkeyReply, String> {
        use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _};
        let mut line = String::new();
        self.reader
            .read_line(&mut line)
            .await
            .map_err(|error| error.to_string())?;
        if line.is_empty() {
            return Err("cache: connection closed".into());
        }
        let marker = line.chars().next().unwrap_or(' ');
        let rest = line[1..].trim_end().to_string();
        Ok(match marker {
            '+' => ValkeyReply::Simple(rest),
            '-' => return Err(format!("cache: {rest}")),
            ':' => ValkeyReply::Integer(rest.parse().unwrap_or(0)),
            '$' => {
                let length: i64 = rest.parse().unwrap_or(-1);
                if length < 0 {
                    ValkeyReply::Nil
                } else {
                    let mut buffer = vec![0u8; length as usize + 2];
                    self.reader
                        .read_exact(&mut buffer)
                        .await
                        .map_err(|error| error.to_string())?;
                    buffer.truncate(length as usize);
                    ValkeyReply::Bulk(Some(String::from_utf8_lossy(&buffer).to_string()))
                }
            }
            '*' => {
                let count: i64 = rest.parse().unwrap_or(0);
                let mut items = Vec::new();
                for _ in 0..count.max(0) {
                    items.push(self.read_reply().await?);
                }
                ValkeyReply::Array(items)
            }
            _ => ValkeyReply::Nil,
        })
    }
}

enum ValkeyReply {
    Simple(String),
    Bulk(Option<String>),
    Integer(i64),
    Array(Vec<ValkeyReply>),
    Nil,
}

#[cfg(test)]
mod valkey_tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};

    async fn read_command(reader: &mut tokio::io::BufReader<tokio::net::TcpStream>) -> Vec<String> {
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let count: usize = line[1..].trim().parse().unwrap();
        let mut command = Vec::new();
        for _ in 0..count {
            line.clear();
            reader.read_line(&mut line).await.unwrap();
            let length: usize = line[1..].trim().parse().unwrap();
            let mut argument = vec![0; length + 2];
            reader.read_exact(&mut argument).await.unwrap();
            argument.truncate(length);
            command.push(String::from_utf8(argument).unwrap());
        }
        command
    }

    #[tokio::test]
    async fn cancelled_commands_do_not_reuse_pending_replies() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("redis://{}", listener.local_addr().unwrap());
        let (requested, received) = tokio::sync::oneshot::channel();
        let (resume, cancelled) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut first = tokio::io::BufReader::new(stream);
            assert_eq!(read_command(&mut first).await, ["GET", "first"]);
            requested.send(()).unwrap();
            cancelled.await.unwrap();
            let payload =
                r#"{"status":200,"final_url":"https://first.invalid/","body":"Zmlyc3Q="}"#;
            let _ = first
                .get_mut()
                .write_all(format!("${}\r\n{payload}\r\n", payload.len()).as_bytes())
                .await;

            let (stream, _) = listener.accept().await.unwrap();
            let mut second = tokio::io::BufReader::new(stream);
            assert_eq!(read_command(&mut second).await, ["GET", "second"]);
            let payload =
                r#"{"status":200,"final_url":"https://second.invalid/","body":"c2Vjb25k"}"#;
            second
                .get_mut()
                .write_all(format!("${}\r\n{payload}\r\n", payload.len()).as_bytes())
                .await
                .unwrap();
        });
        let cache = std::sync::Arc::new(Cache::valkey(&url, Duration::from_secs(60)));
        let first_cache = std::sync::Arc::clone(&cache);
        let first = tokio::spawn(async move { first_cache.get("first").await });
        received.await.unwrap();
        first.abort();
        assert!(first.await.unwrap_err().is_cancelled());
        resume.send(()).unwrap();
        let response = tokio::time::timeout(Duration::from_secs(2), cache.get("second"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response.body.as_slice(), b"second");
        assert_eq!(response.final_url, "https://second.invalid/");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn cache_reads_and_writes_respect_the_fetch_deadline() {
        use crate::engine::EngineRequest;
        use crate::network::{Network, NetworkError};
        use axum::{Router, routing::get};

        let http = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", http.local_addr().unwrap());
        let http_server = tokio::spawn(async move {
            axum::serve(
                http,
                Router::new().route("/", get(|| async { "fresh response" })),
            )
            .await
            .unwrap();
        });
        let network = Network::from_settings(&crate::settings::Settings::defaults()).unwrap();
        let mut outcomes = Vec::new();
        for stall_write in [false, true] {
            let redis = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let cache = Cache::valkey(
                &format!("redis://{}", redis.local_addr().unwrap()),
                Duration::from_secs(60),
            );
            let server = tokio::spawn(async move {
                let (stream, _) = redis.accept().await.unwrap();
                let mut stream = tokio::io::BufReader::new(stream);
                assert_eq!(read_command(&mut stream).await[0], "GET");
                if stall_write {
                    stream.get_mut().write_all(b"$-1\r\n").await.unwrap();
                    assert_eq!(read_command(&mut stream).await[0], "SET");
                }
                std::future::pending::<()>().await;
            });
            let response = tokio::time::timeout(
                Duration::from_secs(2),
                network.fetch(
                    EngineRequest::get(&url),
                    Duration::from_millis(100),
                    Some(&cache),
                    None,
                ),
            )
            .await;
            outcomes.push((stall_write, response));
            server.abort();
        }
        http_server.abort();
        assert!(
            outcomes.iter().all(|(_, response)| response.is_ok()),
            "cache exceeded request budget: {outcomes:?}"
        );
        for (stall_write, response) in outcomes {
            if stall_write {
                assert_eq!(response.unwrap().unwrap().text(), "fresh response");
            } else {
                assert!(matches!(response.unwrap(), Err(NetworkError::Timeout)));
            }
        }
    }

    #[test]
    fn responses_round_trip_through_the_valkey_payload() {
        let response = CachedResponse {
            status: 200,
            final_url: "https://example.test/".into(),
            headers: [("content-type".to_string(), "text/html".to_string())]
                .into_iter()
                .collect(),
            body: std::sync::Arc::new(b"hello".to_vec()),
        };
        let payload = encode_response(&response).expect("encode");
        let decoded = decode_response(&payload).expect("decode");
        assert_eq!(decoded.status, 200);
        assert_eq!(decoded.final_url, "https://example.test/");
        assert_eq!(decoded.body.as_slice(), b"hello");
        assert_eq!(
            decoded.headers.get("content-type").map(String::as_str),
            Some("text/html")
        );
    }
}
