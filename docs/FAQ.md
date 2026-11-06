# FAQ: what RustXNG does and what the benchmarks mean

## Is RustXNG related to SearXNG?

Yes. RustXNG is a Rust implementation based on SearXNG's engine behavior and compatible HTTP conventions.
It is not an official replacement endorsed by SearXNG, and not simply a Rust binding around a running Python service.
See [NOTICE](../NOTICE), [compatibility](COMPATIBILITY.md), and the AGPL-3.0-or-later [license](../LICENSE).

A public Rust library is available for embedding, while the JSON HTTP API is convenient across languages.
Direct library users must manage the async runtime and settings and pin the version they depend on;
this pre-1.0 project does not promise a stable Rust API across releases.

## Does writing it in Rust produce more accurate answers?

No. Implementation language can affect startup, memory and execution overhead; it does not make upstream facts true.
Source selection, query handling, ranking, deduplication and failure handling can change retrieval quality independently.
RustXNG returns search evidence; a language model's answer accuracy is a separate measurement.

## Does local hosting remove all search costs and rate limits?

No. RustXNG itself does not charge per search request, but servers, networking and optional paid providers still have costs.
Standard and Full send queries to external engines, which can rate-limit, block, require credentials or change their response formats. Clean searches prebuilt local indexes only; explicitly enabled raw-page retrieval or other network-backed features are separate.
Many parallel agents can increase those problems. Caching, bounded concurrency and suitable engine selection help;
local hosting does not bypass provider limits.

## Is public Internet connectivity enough?

It is necessary for external search, but not sufficient. DNS, TLS, network region/IP reputation, provider availability,
credentials and quotas affect results. `/healthz` only shows that the local process is responding.
Inspect search results and `unresponsive_engines`; do not treat an HTTP 200 as proof of useful retrieval.

## Is it as accurate as Exa or another commercial API?

Only a comparison with stated datasets, dates and conditions can support that claim.
Earlier [benchmark reports](BENCHMARK.md) compare specific workloads; their scores are not a universal ranking.
The [SealQA run](SEALQA.md) measures reference-page retrieval, not generated-answer correctness.
Keep errors and misses in the denominator, record cache/local-index settings, and repeat runs before drawing conclusions.

## What about full Japanese error messages?

The [November 5 Japanese troubleshooting observations](../bench/results/issue-resolution-2026-11-05/japanese/baseline/summary.json) show poor reference-page recall in a specific older API-backed test configuration. They do not establish the current v0.30.0 performance for that workload.
Full error text, a shortened query and quoted exact text can behave differently; language handling and source coverage matter.
Use the benchmark report form with a public error example and an expected source URL.
Japanese technical retrieval and broad/current-information collection are tracked in [GitHub Issues](https://github.com/sahenjp/RustXNG/issues).

A pinpoint troubleshooting task can prioritize one authoritative matching page and a small result count.
A broad current-information task needs multiple independent sources, coverage, freshness and missing-source reporting.
A date filter only works when the selected provider supports it at the requested precision.
Neither a high hit@1 score nor a large result count alone measures both tasks.

## Won't fetched page text fill the model context?

It can. Start with `max_results=5`, relevant domains, and short snippets. Project the fields passed to the model locally
when appropriate. Page fetching is optional; review the [current security policy](../SECURITY.md) and the [historical audit](AUDIT-2026-11-02.md).
MCP provides character/byte budgets and up to three query-matched passages from the extracted page. An explicit `tokenizer` (`cl100k_base` or `o200k_base`) together with `max_payload_tokens` also limits serialized `structuredContent`, but does not include the JSON-RPC envelope or the caller's prompt. See [the MCP budget contract](AGENT-OUTPUT.md). HTTP compression saves transfer bytes, not model tokens.

## Does it support Linux ARM?

Linux ARM64 (`aarch64-unknown-linux-gnu`, glibc 2.39+) binaries are available from releases after v0.26.0, including v0.28.0.
Linux ARMv7 and Linux ARM64 musl are separate, unsupported binary targets. Check the current [release assets](https://github.com/sahenjp/RustXNG/releases) and checksum for your platform; macOS ARM64 binaries do not run on Linux.

## How can I help or become a maintainer?

You can submit a report, fixture, benchmark reproduction, documentation change or PR without an invitation.
Start at [CONTRIBUTING.md](../CONTRIBUTING.md). For ongoing responsibility,
use the maintainer application form and read [GOVERNANCE.md](../GOVERNANCE.md).
Praise, follower counts and an application do not automatically grant repository access.
