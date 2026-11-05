//! Export ranked local-index results without contacting external providers.
//! Usage: cargo run --release --example local_quality -- INDEX QUERIES.json
use std::path::Path;
use std::time::Instant;

use rustxng::engine::EngineItem;
use rustxng::localindex::LocalIndex;
use rustxng::results::{ResultContainer, SearchResult, query_terms};
use rustxng::settings::Settings;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: local_quality INDEX QUERIES.json".into());
    }
    let settings = Settings::defaults();
    let index = LocalIndex::open(Path::new(&args[1]))?;
    let data: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&args[2])?)?;
    let queries = data["queries"]
        .as_array()
        .ok_or("expected a queries array")?;
    let mut rows = Vec::new();
    for row in queries {
        let query = row["query"].as_str().ok_or("expected a query string")?;
        let started = Instant::now();
        let items = index
            .search(query, 20)?
            .into_iter()
            .map(|hit| {
                EngineItem::Result(SearchResult::new(hit.url, hit.title).with_content(hit.snippet))
            })
            .collect();
        let mut results = ResultContainer::new();
        results.extend("local_index", 1.0, items);
        results.close(
            |_| 1.0,
            &query_terms(query),
            settings.search.relevance_weight,
            query,
            settings.search.authority_weight,
        );
        rows.push(serde_json::json!({
            "id": row["id"], "query": query, "gold": row["gold"],
            "millis": started.elapsed().as_secs_f64() * 1000.0,
            "urls": results.ordered().iter().take(10).map(|r| &r.url).collect::<Vec<_>>()
        }));
    }
    println!("{}", serde_json::json!({"index": args[1], "rows": rows}));
    Ok(())
}
