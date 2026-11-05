//! Reproducible, offline network/cache benchmark: no upstream services contacted.
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::{Router, routing::get};
use rustxng::cache::Cache;
use rustxng::engine::EngineRequest;
use rustxng::network::Network;
use rustxng::settings::Settings;

#[tokio::main]
async fn main() {
    const REQUESTS: usize = 16;
    const ROUNDS: usize = 5;
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let app = Router::new().route(
        "/",
        get(move |uri: axum::http::Uri| {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::Relaxed);
                tokio::time::sleep(Duration::from_millis(40)).await;
                format!("result for {uri}")
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut report = serde_json::Map::new();
    for (name, identical) in [("identical", true), ("distinct", false)] {
        let mut latencies = Vec::new();
        let mut counts = Vec::new();
        for _ in 0..ROUNDS {
            let mut settings = Settings::defaults();
            settings.outgoing.host_spacing_ms = 20;
            settings.outgoing.retries = 0;
            let network = Network::from_settings(&settings).unwrap();
            let cache = Cache::default();
            calls.store(0, Ordering::Relaxed);
            let started = Instant::now();
            let requests = (0..REQUESTS).map(|i| {
                let request =
                    EngineRequest::get(format!("{url}?q={}", if identical { 0 } else { i }));
                network.fetch(request, Duration::from_secs(2), Some(&cache), None)
            });
            let responses = futures::future::join_all(requests).await;
            latencies.push(started.elapsed().as_secs_f64() * 1000.0);
            for (i, response) in responses.into_iter().enumerate() {
                assert_eq!(
                    response.unwrap().text(),
                    format!("result for /?q={}", if identical { 0 } else { i })
                );
            }
            counts.push(calls.load(Ordering::Relaxed));
        }
        latencies.sort_by(f64::total_cmp);
        report.insert(
            name.into(),
            serde_json::json!({
                "requests_per_round": REQUESTS,
                "rounds": ROUNDS,
                "median_ms": latencies[ROUNDS / 2],
                "upstream_requests_per_round": counts,
            }),
        );
    }
    println!("{}", serde_json::Value::Object(report));
    server.abort();
}
