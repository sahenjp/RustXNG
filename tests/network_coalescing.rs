use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::{Router, routing::get};
use futures::future::join_all;
use rustxng::cache::Cache;
use rustxng::engine::EngineRequest;
use rustxng::network::{Network, NetworkError};
use rustxng::settings::Settings;

async fn serve(app: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, task)
}

fn network() -> Network {
    let mut settings = Settings::defaults();
    settings.outgoing.retries = 0;
    Network::from_settings(&settings).unwrap()
}

#[tokio::test]
async fn concurrent_identical_cache_misses_share_one_upstream_response() {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let (url, server) = serve(Router::new().route(
        "/",
        get(move || {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::Relaxed);
                tokio::time::sleep(Duration::from_millis(50)).await;
                "same result"
            }
        }),
    ))
    .await;
    let network = network();
    let cache = Cache::default();
    let responses = join_all((0..16).map(|_| {
        network.fetch(
            EngineRequest::get(&url),
            Duration::from_secs(2),
            Some(&cache),
            None,
        )
    }))
    .await;
    for response in responses {
        assert_eq!(response.unwrap().text(), "same result");
    }
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    server.abort();
}

#[tokio::test]
async fn different_headers_are_independent_even_for_the_same_url() {
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let (url, server) = serve(Router::new().route(
        "/",
        get(move |headers: axum::http::HeaderMap| {
            let barrier = barrier.clone();
            async move {
                barrier.wait().await;
                headers["accept-language"].to_str().unwrap().to_owned()
            }
        }),
    ))
    .await;
    let network = network();
    let cache = Cache::default();
    let responses = join_all(["ja", "en"].map(|lang| {
        network.fetch(
            EngineRequest::get(&url).header("Accept-Language", lang),
            Duration::from_secs(2),
            Some(&cache),
            None,
        )
    }))
    .await;
    let texts: Vec<_> = responses.into_iter().map(|r| r.unwrap().text()).collect();
    assert_eq!(texts, ["ja", "en"]);
    server.abort();
}

#[tokio::test]
async fn a_timed_out_waiter_does_not_cancel_the_upstream_request() {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let (url, server) = serve(Router::new().route(
        "/",
        get(move || {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::Relaxed);
                tokio::time::sleep(Duration::from_millis(100)).await;
                "result"
            }
        }),
    ))
    .await;
    let network = network();
    let cache = Cache::default();
    let (leader, waiter) = tokio::join!(
        network.fetch(
            EngineRequest::get(&url),
            Duration::from_secs(2),
            Some(&cache),
            None
        ),
        network.fetch(
            EngineRequest::get(&url),
            Duration::from_millis(20),
            Some(&cache),
            None
        ),
    );
    assert_eq!(leader.unwrap().text(), "result");
    assert!(matches!(waiter, Err(NetworkError::Timeout)));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    server.abort();
}

#[tokio::test]
async fn a_cancelled_leader_releases_the_request_for_a_waiter() {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let (url, server) = serve(Router::new().route(
        "/",
        get(move || {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::Relaxed);
                tokio::time::sleep(Duration::from_millis(100)).await;
                "result"
            }
        }),
    ))
    .await;
    let network = network();
    let cache = Cache::default();
    let (leader, waiter) = tokio::join!(
        network.fetch(
            EngineRequest::get(&url),
            Duration::from_millis(30),
            Some(&cache),
            None
        ),
        network.fetch(
            EngineRequest::get(&url),
            Duration::from_secs(2),
            Some(&cache),
            None
        ),
    );
    assert!(matches!(leader, Err(NetworkError::Timeout)));
    assert_eq!(waiter.unwrap().text(), "result");
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    server.abort();
}

#[tokio::test]
async fn failed_responses_are_retried_by_the_next_caller() {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let (url, server) = serve(Router::new().route(
        "/",
        get(move || {
            let first = count.fetch_add(1, Ordering::Relaxed) == 0;
            async move {
                tokio::time::sleep(Duration::from_millis(30)).await;
                if first {
                    (axum::http::StatusCode::SERVICE_UNAVAILABLE, "unavailable")
                } else {
                    (axum::http::StatusCode::OK, "result")
                }
            }
        }),
    ))
    .await;
    let network = network();
    let cache = Cache::default();
    let (first, next) = tokio::join!(
        network.fetch(
            EngineRequest::get(&url),
            Duration::from_secs(2),
            Some(&cache),
            None
        ),
        network.fetch(
            EngineRequest::get(&url),
            Duration::from_secs(2),
            Some(&cache),
            None
        ),
    );
    assert!(matches!(
        first,
        Err(NetworkError::Status { status: 503, .. })
    ));
    assert_eq!(next.unwrap().text(), "result");
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    server.abort();
}

#[tokio::test]
async fn host_spacing_and_transport_share_one_deadline() {
    let (url, server) = serve(Router::new().route(
        "/",
        get(|| async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            "result"
        }),
    ))
    .await;
    let mut settings = Settings::defaults();
    settings.outgoing.host_spacing_ms = 400;
    settings.outgoing.retries = 0;
    let network = Network::from_settings(&settings).unwrap();
    network
        .fetch(EngineRequest::get(&url), Duration::from_secs(2), None, None)
        .await
        .unwrap();
    // The next slot is ~200 ms away. A second 200 ms transport cannot fit
    // inside a single 300 ms budget, even though each wait separately fits.
    let result = network
        .fetch(
            EngineRequest::get(&url),
            Duration::from_millis(300),
            None,
            None,
        )
        .await;
    assert!(matches!(result, Err(NetworkError::Timeout)));
    server.abort();
}
