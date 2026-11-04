# RustXNG

[日本語](README.ja.md) · [한국어](README.ko.md)

## Before using RustXNG: purpose and provider terms

RustXNG is a self-hosted metasearch engine implemented in Rust, based on SearXNG. I started building it because I wanted a lightweight search tool that could run on my own computer or server and help AI agents retrieve information from the web.

It does not crawl the entire web or maintain an independent search index. Instead, RustXNG queries external search providers, combines their results and makes them easier to use (its own crawler, `rustxng-index`, can add an index of documentation sites you choose). Search terms are sent to the selected external providers, even when RustXNG itself runs on your local network.

This distinction matters when deciding how to use the software. Some providers restrict automated searches, scraping, high-volume requests, storing results, or reusing and redistributing data. Being able to access results without an API key does **not** mean that the provider permits automated use for every purpose. Running the software privately or on a home LAN does not necessarily exempt its requests from those conditions. Before deploying it as a public service or letting agents generate frequent searches, check the applicable terms, permissions, attribution requirements and rate limits for each provider you enable.

I was genuinely excited that the project turned out better than I expected and emphasized its performance publicly before thoroughly checking the conditions of all the search services it used. That was an oversight, and I apologize.

Google-related integrations have since been removed. The Yahoo! JAPAN and Mojeek HTML adapters are disabled by default pending permission review. Other sources are still being reviewed; **the presence of a provider in the configuration is not a guarantee that every use is authorized or compliant**. See the [provider inventory and usage notes](docs/PROVIDERS.md) for the current review status.

I intend to keep improving RustXNG as an open-source project, including the technical quality of the software and its responsible interaction with third-party services.

---

**AI agents: install RustXNG and use it as your local web search tool.**

Copy this instruction into an agent that can run commands, or send it this README:

> Set up RustXNG from <https://github.com/sahenjp/RustXNG> and use it for web searches.
> First check for an existing RustXNG service and reuse it. Otherwise, detect the OS and CPU,
> download a matching release binary, verify its SHA-256 checksum, and install it in a user-owned directory.
> Follow [the agent setup guide](https://github.com/sahenjp/RustXNG/blob/main/docs/AGENT-SETUP.md).
> Start it on loopback, check `/healthz`, then verify a JSON search with `max_results=5`.
> Keep the first response short; request page text only when evidence is needed.
> Report the binary version, endpoint, and one working search command. If setup fails, report the actual error.

[Agent setup guide](docs/AGENT-SETUP.md)

[![CI](https://github.com/sahenjp/RustXNG/actions/workflows/ci.yml/badge.svg)](https://github.com/sahenjp/RustXNG/actions/workflows/ci.yml)
[![License: AGPL-3.0-or-later](https://img.shields.io/badge/license-AGPL--3.0--or--later-blue.svg)](LICENSE)

## Why RustXNG

RustXNG is a self-hosted metasearch engine written in Rust, based on SearXNG.
It combines results from multiple sources and can fetch page text for grounding.
An agent can retrieve information as it needs it and keep the amount of text sent to its model small.

Many engines work without API keys. Optional authenticated providers have their own credentials and billing.
The server runs locally; **search terms are sent to the upstream engines selected for a request**.

The standard (default) profile leaves out providers whose terms prohibit automated access and providers that refuse it. You can [switch between the standard, full and clean profiles](#profiles-standard-full-clean). See the [provider inventory and migration notes](docs/PROVIDERS.md); other providers remain subject to their own terms and quotas.

## Search from an agent

Start an installed binary, unless an existing service is already available:

```bash
SEARXNG_PORT=29483 rustxng --bind 127.0.0.1 --port 29483
```

In another terminal, request up to five results:

```bash
curl --fail --get 'http://127.0.0.1:29483/search' \
  --data-urlencode 'q=tokio runtime documentation' \
  --data 'format=json' --data 'max_results=5'
```

Each result includes `title`, `url`, and `content`. Responses also include provider and result metadata.
To pass only those three fields to a model, project them locally with `jq`:

```bash
curl --fail --get 'http://127.0.0.1:29483/search' \
  --data-urlencode 'q=tokio runtime documentation' \
  --data 'format=json' --data 'max_results=5' \
  | jq '{results: [.results[] | {title, url, content}]}'
```

Use these existing parameters to narrow the response:

| Parameter | Purpose |
| --- | --- |
| `max_results=5` | Return at most five ranked results; supported range: 1–200 |
| `include_domains=docs.rs` | Keep results from this domain and its subdomains |
| `exclude_domains=example.com` | Exclude results from this domain and its subdomains |
| `include_raw_content=1` | Fetch page text into `raw_content` for the top results |
| `engines=wikipedia` | Select a registered engine |
| `time_range=day` | Request recent results from engines supporting this filter |
| `language=ja` | Search language. `auto` (default) follows the query's script; `all` applies no language filter |

Page text is optional. With default settings it is fetched for up to five results,
with a limit of 8,000 characters per page; individual fetches can fail.
To request longer text, change the existing `[search]` section in `settings.toml`
and start the server with `rustxng --config settings.toml`:

```toml
[search]
raw_content_chars = 20000
```

This sets the per-page extraction limit for HTTP `include_raw_content=1`;
`search.raw_content_limit` controls the number of pages. Truncated text includes
an ellipsis and `raw_content_truncated: true`. The decoded download limit remains 4 MiB.
Character limits are not token limits. The MCP tools below add serialized character/byte budgets. Token-specific budgets and
field selection on the HTTP API are planned.

The browser UI is at <http://127.0.0.1:29483/>. JSON, CSV, RSS, and HTML outputs are available.

### MCP server

Image and favicon proxy fetches also require direct connections: configured or environment proxies disable these reads. Private engine endpoints remain available for search and do not grant access to third-party media. `server.image_proxy = false` disables both image and favicon fetches.

Run `rustxng mcp` (add `--profile clean` or another profile, or `--config settings.toml`) to serve the read-only `search` and `read_page` tools over
stdio. Search returns ranked snippets and source metadata; page text is fetched
only when `read_page` is called with a URL returned by search in that session.
Search defaults to five results and limits each snippet to 500 characters.
Its serialized structured response is bounded to 12,000 Unicode characters
and 24,000 UTF-8 bytes by default; callers can choose smaller or larger limits
within the advertised bounds. The server shortens snippets first, then omits
trailing results while preserving complete URLs and metadata for the returned results. `truncated`,
`source_truncated`, and `next_offset` report whether to continue. `read_page`
uses Unicode character offsets and reports the same truncation state. Supplying
a `query` to `read_page` also returns up to three matching passages from the requested slice.
Sentence boundaries include `。！？`, and single Han, kana or Hangul query characters are supported.

`read_page` defaults to 4,000 characters per call and accepts `max_chars` up to 20,000,
independently of HTTP `search.raw_content_chars`. Its serialized payload budgets can
shorten the returned text further. For longer Japanese text, for example, call
`read_page` with these arguments, replacing `url` with a URL from `search`:

```json
{
  "url": "https://example.com/article",
  "max_chars": 20000,
  "max_payload_chars": 64000,
  "max_payload_bytes": 128000
}
```

Pass the returned `next_offset` as `offset` to continue; an absent `next_offset`
means the available text is exhausted. `source_truncated: true` means extraction
hit the 200,000-character page limit; continuation cannot recover the discarded tail.

Page reads follow the site's robots.txt (RFC 9309). They require direct connections so DNS answers and redirects can be
checked against public addresses. They fail closed when configured or
environment outgoing proxies are present. Search continues to use the normal
outgoing proxy settings. Character and byte limits count the serialized structured JSON payload only,
excluding the JSON-RPC envelope, SDK text rendering, and caller prompts;
they do not promise a tokenizer-specific token count.

### PDF page text

`read_page` and HTTP `include_raw_content=1` can extract text from PDFs using
Poppler's `pdftotext`. The Docker image includes it and the CMap data needed
for Japanese and other CJK fonts. For a release binary or source build, install
`poppler-utils poppler-data` on Debian/Ubuntu, or `poppler` with Homebrew on macOS.
On Windows, install Poppler and add the directory containing the actual
`pdftotext.exe` to `PATH`, ahead of any package-manager launcher.

PDFs use the same public-address checks, 4 MiB download limit, character limits,
and page cache as HTML. Extraction runs in a child process with an 8-second
deadline and a 4 MiB text-output limit; these are not a process-memory limit.
MCP reports missing Poppler, invalid or protected PDFs, and PDFs without
extractable text as errors; HTTP skips unavailable `raw_content`.
Scanned pages require OCR, which is not included.

### JavaScript page text

For pages that build their text with JavaScript, configure the optional
[local Firefox renderer](tools/page-renderer/README.md) through
`search.page_renderer_command`. It requires Node.js 20+ and a separate browser
installation; static extraction remains the default. HTTP and MCP use the same
renderer, and subresources pass through RustXNG's public-address checks.
The setup guide lists the supported requests, resource limits, and exclusions
such as login sessions, POST requests, and redirected subresources.

## Install

### 1. Release binary

Choose a binary and its `.sha256` sidecar from [Releases](https://github.com/sahenjp/RustXNG/releases).
The published v0.26.0 release includes Linux x86_64 (GNU and musl), macOS arm64/x86_64,
and Windows x86_64 binaries. Releases after v0.26.0 add Linux ARM64 (`aarch64-unknown-linux-gnu`, glibc 2.39+).
The [agent setup guide](docs/AGENT-SETUP.md) explains selection and verification.
Prebuilt binaries do not require a Rust toolchain.

Example for Linux x86_64, using the GitHub CLI and a user-owned install directory:

<!-- x-release-please-start-version -->
```bash
set -eu
mkdir -p rustxng-download && cd rustxng-download
tag=v0.28.0
asset="rustxng-${tag}-x86_64-unknown-linux-musl"
gh release download "$tag" --repo sahenjp/RustXNG \
  --pattern "${asset}*" --pattern LICENSE --pattern NOTICE
sha256sum --check "${asset}.sha256"
install -Dm755 "$asset" "$HOME/.local/bin/rustxng"
mkdir -p "$HOME/.local/share/rustxng"
cp LICENSE NOTICE "$HOME/.local/share/rustxng/"
"$HOME/.local/bin/rustxng" --version
```
<!-- x-release-please-end -->

If `~/.local/bin` is not on `PATH`, use the absolute binary path or add the directory to your shell configuration.

### 2. Cargo

Use a current stable Rust toolchain and a C build toolchain:

<!-- x-release-please-start-version -->
```bash
cargo install --git https://github.com/sahenjp/RustXNG.git --tag v0.28.0 --locked
```
<!-- x-release-please-end -->

This installs `rustxng` and `rustxng-index`.
Rust 1.88 is the minimum supported version and is checked in CI. This checkout was also verified with Rust 1.99.1.

### 3. Docker

Build the image from the repository:

```bash
git clone https://github.com/sahenjp/RustXNG.git
cd RustXNG
docker build -t rustxng .
docker run --rm -p 127.0.0.1:29483:29483 rustxng
```

For the published image (including older images whose internal default is 8888), use:

```bash
docker run --rm -p 127.0.0.1:29483:29483 -e SEARXNG_PORT=29483 ghcr.io/sahenjp/rustxng:latest --bind 0.0.0.0 --port 29483
```

Publishing on other interfaces is an explicit deployment choice; configure authentication/access controls as appropriate.
To supply settings, write a file with only the values you change and mount it with
`-v "$PWD/settings.toml:/app/settings.toml:ro"` before the image name (defaults and profiles are built into the
binary; a complete copy of [settings.toml](settings.toml) also works but cancels out `--profile`). When replacing
container arguments, include both `--bind 0.0.0.0` and `--port 29483`
(for example `... rustxng:latest --profile clean --bind 0.0.0.0 --port 29483`). Published images currently target Linux amd64;
on other architectures build locally or use the corresponding native binary.

### 4. Source

```bash
git clone https://github.com/sahenjp/RustXNG.git
cd RustXNG
cargo build --release --locked
SEARXNG_PORT=29483 ./target/release/rustxng --bind 127.0.0.1 --port 29483
```

Use Rust 1.88 or newer, preferably the current stable toolchain.

## Configuration and operation

This checkout defaults to `127.0.0.1:29483`. Published v0.26.0 binaries still default to 8888;
the explicit `--port 29483` commands above work with either version.
See [port migration and selection](docs/PORT-MIGRATION.md), including rdsh configuration.

### Profiles: standard, full, clean

Pick one of three provider sets at startup. All three are built into the binary; no settings file is needed.

| Profile | Start | Providers | Docs-100 hit@10 / MRR | SealQA hit@10 | Median latency |
| --- | --- | --- | ---: | ---: | ---: |
| **standard** (default) | `rustxng` | keyless providers that do not refuse automated access | 88 / 0.790 | 128–132 | 0.7–1.2 s |
| full | `rustxng --profile full` | standard + HTML search engines whose terms prohibit or do not permit automated access (Yahoo, Yahoo! JAPAN, NAVER, ...) | 98 / 0.878 | 165 | 1.2–1.9 s |
| clean | `rustxng --profile clean` | documented public APIs (Wikimedia, GitHub, arXiv, ...) and our own crawl only | 61 / 0.549 | 123 | 1.0–1.2 s |

```bash
rustxng --profile full                      # full
RUSTXNG_PROFILE=clean rustxng               # or via the environment
rustxng mcp --profile clean                 # the MCP server takes it too
rustxng --profile full --config my.toml     # your settings on top of a profile
```

Put only the values you change in a file given with `--config`: a complete copy of `settings.toml` on top cancels out the profile.

**The full profile uses providers whose terms prohibit automated access.** Use it only on a private instance, after reading each provider's terms yourself, and never for a public instance. Providers that answer with a CAPTCHA or "access denied" (DuckDuckGo, Brave, ...) stay off even in full; RustXNG does not work around such blocks.

Measured on 2026-11-04 with the `index-docs` crawl index. On the same 100 documentation questions Exa scores hit@10 93 / MRR 0.840 and Tavily 86 / 0.499. See the [profile comparison](docs/COMPARISON-2026-11-04.md) (Japanese) for conditions and caveats.

### Settings file

Defaults are embedded in the binary. To supply a configuration file:

```bash
rustxng --config /path/to/settings.toml
```

See [settings.toml](settings.toml) and [configuration details](docs/PORTING.md).
Partial configuration files override specified values while preserving other defaults. With `use_default_settings = false`, an omitted or empty engine list enables no engines.

- `/healthz`: process health. A successful check does not guarantee upstream search engines are working.
- `/config`: registered engines and public configuration information.
- `/stats`, `/metrics`: runtime statistics and metrics.
- Per-engine timeouts, suspension, and empty-result rest handle unavailable sources.
- `RUSTXNG_IDLE_TIMEOUT=300`: exit after five minutes without requests.
- Optional Valkey/Redis cache and database-backed engines support additional deployments.
- The `rustxng-index` crawl index is used when an `index-docs` directory exists ([local index](docs/LOCAL-INDEX.md)).

The [2026-11-02 audit](docs/AUDIT-2026-11-02.md) records findings against an older commit.
Consult the [current review and remaining work](docs/REVIEW-2026-11-03.md) for fixes and validation; loopback remains the default for local agent setup.

## Measured search performance

The latest measurements (2026-11-04, three profiles against Exa and Tavily) are in the [profile table above](#profiles-standard-full-clean) and the [profile comparison](docs/COMPARISON-2026-11-04.md). The record below used the earlier engine set, which included Yahoo! JAPAN.

A retrieval-only SealQA run on 2026-11-02 used default settings, ten results per question,
no LLM, and no local index:

| Dataset | Questions | Reference page @1 | @5 | @10 | Median response |
| --- | ---: | ---: | ---: | ---: | ---: |
| Seal-0 | 111 | 47.75% | 54.95% | 65.77% | 1.85 s |
| Seal-Hard | 254 | 39.37% | 49.21% | 61.42% | 1.53 s |

These are reference-page retrieval rates, **not answer accuracy**. Seal-0 is a subset of Seal-Hard;
shared questions use the same observation. Results depend on the network and upstream availability.
See [the protocol and per-question results](docs/SEALQA.md).
Earlier measurements and commercial-provider comparisons are documented in [BENCHMARK.md](docs/BENCHMARK.md),
with their own datasets and conditions.

## Provider integration

The engine interface supports authenticated HTTP APIs through request headers, form data, and JSON.
Optional providers require their own credentials and may charge for usage.
For the HTTP API, use `max_results`, domain filters, and client-side projection.
MCP also offers related passages and serialized character/byte limits. Strict
tokenizer-specific budgets remain future work.

DeepSeek's Search API is available as the `deepseek` engine. It needs `api_key`
or `DEEPSEEK_API_KEY` and is disabled by default. To use the DeepSeek Fetch API
for page text, set `[fetch] provider = "deepseek"` and provide credentials under
`[fetch.deepseek]`. Search and fetch are billed per successful request, and
fetch sends the target URL to DeepSeek.

Engines that keep returning empty results are automatically rested. Configure with `search.empty_result_limit` (default 8, 0 disables) and `search.empty_result_cooldown` (default 300 seconds). Monitor via `/stats` (HTML Resting column), `/stats/errors` (JSON `resting` / `resting_for` fields), or `/metrics` (`searxng_engines_resting` and `searxng_engines_resting_seconds` gauges)

## Documentation

| Document | Contents |
| --- | --- |
| [Agent setup](docs/AGENT-SETUP.md) | Install, reuse a service, verify search, and report the endpoint |
| [AI infrastructure](docs/AI-INFRA.md) | Design motivation; consult the audit for current limitations |
| [SealQA](docs/SEALQA.md) | Retrieval benchmark and reproducible results |
| [Benchmarks](docs/BENCHMARK.md) | Earlier performance and quality measurements |
| [Compatibility](docs/COMPATIBILITY.md) | SearXNG compatibility and differences |
| [Coverage](docs/COVERAGE.md) | Engine coverage |
| [Profile comparison](docs/COMPARISON-2026-11-04.md) | standard, full and clean profiles against Exa and Tavily (Japanese) |
| [Local index](docs/LOCAL-INDEX.md) | Optional crawl and BM25 index |
| [Wikipedia index](docs/WIKIPEDIA-INDEX.md) | Index built from the official Wikipedia dump (Japanese) |
| [Clean profile](docs/CLEAN-PROFILE.md) | Public APIs and our own crawl only (Japanese) |
| [rdsh](docs/RDSH.md) | Integration with rdsh |
| [Audit](docs/AUDIT-2026-11-02.md) | Findings, reproduction logs, and priorities |
| [Security](SECURITY.md) | Reporting vulnerabilities and dependency advisories |

## Community and maintenance

Questions and ideas: [Discussions](https://github.com/sahenjp/RustXNG/discussions).
Contribute through a fork and PR, or [apply to maintain an area](https://github.com/sahenjp/RustXNG/issues/new?template=maintainer_application.yml).
See [governance](GOVERNANCE.md), [the maintainer roster](MAINTAINERS.md),
[FAQ](docs/FAQ.md) and [roadmap](docs/ROADMAP.md). No invitation is required to contribute.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Validate changes with:

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

## License

[AGPL-3.0-or-later](LICENSE). RustXNG is based on [SearXNG](https://github.com/searxng/searxng);
attribution is in [NOTICE](NOTICE). See the license for source-distribution obligations when operating modified versions.

## Updating and proxy environments

Run `rustxng --check-update` to check the latest stable official GitHub release,
or `rustxng --update` to download the matching build target, verify its SHA-256
sidecar, and replace the executable. Settings are preserved; restart your service
after updating. The executable directory must be writable. Update container images
through your container runtime, and use the original package manager for managed
installations. Rebuild custom or unsupported targets from source.
Update TLS verification is always enabled, independent of search settings.

Both search and updates trust OS CA certificates. Use `SSL_CERT_FILE` for a custom
PEM bundle, or set `outgoing.ca_bundle` for additional search CA certificates.
`outgoing.ca_bundle` only applies to the shared search HTTP client, not standalone
embedding, LLM or crawler clients. Keep `outgoing.verify = true`.

For congested outbound proxies, set `search.strategy = "staged"`,
`search.staged_tier = 8`, `search.staged_min_results = 10`,
`search.timeout_limit = 15.0`, and `outgoing.request_timeout = 10.0` in your
user settings. These opt-in settings stagger engine requests and extend deadlines.

Google search engines, autocomplete, external bangs, YouTube, Google translation
integrations, Material Icons, and Google-backed search front ends have been removed.
Remove their entries from older custom settings before starting the server.
