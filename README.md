# RustXNG

[日本語](README.ja.md) · [한국어](README.ko.md)

RustXNG is a self-hosted metasearch engine written in Rust, based on [SearXNG](https://github.com/searxng/searxng). It combines results from external search providers and can fetch page text for AI agents through a JSON API or MCP.

RustXNG does not maintain a web-scale search index. You can add a local index of selected sites with `rustxng-index` and import Wikipedia dumps separately.

## Quick start

Install from the [releases page](https://github.com/sahenjp/RustXNG/releases) or with Cargo. Release binaries are available for Linux, macOS and Windows; check the matching asset and its SHA-256 sidecar before running it.

<!-- x-release-please-start-version -->
```bash
cargo install --git https://github.com/sahenjp/RustXNG.git --tag v0.30.0 --locked
```
<!-- x-release-please-end -->

Rust 1.88 or newer and a C build toolchain are required for source installation. The [agent setup guide](docs/AGENT-SETUP.md) covers release binaries, verification, platform requirements and service reuse.

Start a local server:

```bash
rustxng --bind 127.0.0.1 --port 29483
```

Search from another terminal:

```bash
curl --fail --get 'http://127.0.0.1:29483/search' \
  --data-urlencode 'q=tokio runtime documentation' \
  --data 'format=json' --data 'max_results=5'
```

Results contain a title, URL, snippet and source metadata. For automated clients that need errors for unknown engine names, add `strict_engines=1` (the default preserves SearXNG-compatible fallback behavior). Useful optional parameters include `language=ja`, `engines=wikipedia`, `include_domains=docs.rs`, `exclude_domains=example.com`, `time_range=day`, and `include_raw_content=1`. Time filtering depends on provider support.

The web interface is at <http://127.0.0.1:29483/>. The search endpoint also supports CSV, RSS and HTML output.

## Use from an AI agent

Start the built-in MCP server:

```bash
rustxng mcp
```

MCP exposes two read-only tools:

- `search` returns up to 10 ranked results. The default is 5, with snippets limited to 500 characters.
- `read_page` retrieves text from a URL returned by `search` in the same session. It can return up to three passages relevant to an optional `query`, selected from the extracted page.

`read_page` returns 4,000 characters by default and allows `max_chars` up to 20,000. Use `offset` and `next_offset` to continue. `truncated` and `source_truncated` indicate limits. MCP enforces serialized character and byte budgets. Supply both `tokenizer` (`cl100k_base` or `o200k_base`) and `max_payload_tokens` for an exact budget on serialized MCP `structuredContent`; this does **not** include the JSON-RPC envelope or client/model framing. [Output contract](docs/AGENT-OUTPUT.md)

Page fetching checks destinations and redirects for non-public addresses and follows robots.txt (RFC 9309). It does not accept arbitrary URLs through MCP. Treat all returned web text as untrusted content.

Use `accept_language`, for example `"ko-KR,ko;q=0.9,en;q=0.8"`, to request a page-language preference in MCP `read_page` or HTTP `include_raw_content`. Omission sends no language header; each preference has its own cached page. See [validation, providers, and pagination](docs/COMPATIBILITY.md#ページ本文の言語).

For page text, `include_raw_content=1` uses the HTTP API. PDF text extraction requires Poppler (`pdftotext`); scanned PDFs require a separate OCR tool. JavaScript-rendered pages need the optional [Firefox renderer](tools/page-renderer/README.md). Neither is enabled automatically by a standalone release binary.

## Search profiles

Choose a provider configuration with `--profile`. The default is `standard`.

| Profile | Command | Sources |
| --- | --- | --- |
| Standard | `rustxng` | 34 keyless entries with recorded endpoint/use conditions, plus local indexes |
| Clean | `rustxng --profile clean` | Search only prebuilt local crawl and Wikipedia-dump indexes; no external search-provider requests |
| Full | `rustxng --profile full` | Additional HTML providers, including sources with restrictive or unverified automation terms, plus billed Exa Search (skipped at startup when `EXA_API_KEY` is absent) |

**Full is not a permission bypass.** Do not enable a provider without the permissions required for your use. A private instance does not override provider terms. Clean is a configuration choice, not a certification of legal compliance.

The selected profile also works with MCP, for example `rustxng mcp --profile clean`. Standard and Full send queries to their enabled external providers even when RustXNG runs locally. Clean searches local indexes only, but explicitly requested raw page fetching or other network-backed features can still make outbound requests. Full's Exa integration is opt-in and billed; an API key alone does not enable Exa in Standard.

Standard excludes unreviewed providers from its defaults. Reviews apply to the documented retrieval scope, **not** every commercial use, content licence, storage policy or redistribution scenario. See [provider conditions](docs/PROVIDERS.md) and [dated measurements](docs/ISSUE-RESOLUTION-2026-11-05.md).

## Installation and configuration

A Docker image is available:

```bash
docker run --rm -p 127.0.0.1:29483:29483 \
  -e SEARXNG_PORT=29483 ghcr.io/sahenjp/rustxng:latest \
  --bind 0.0.0.0 --port 29483
```

By default, the service binds to loopback. Exposing it to another network requires appropriate access controls.

Settings are embedded. To override a value, create a small `my-settings.toml`, for example:

```toml
[server]
idle_timeout = 600
```

```bash
rustxng --config my-settings.toml
```

A custom settings file is layered over the selected profile; avoid copying all shipped settings or adding an unintended engine override. Use `rustxng --check-update` to check for a release or `rustxng --update` for a supported binary installation. For managed packages and containers, use the original installation method.

The `/healthz` endpoint checks the process, not the availability of upstream providers. Use a real JSON search to verify that results are returned. Runtime metrics are exposed at `/stats` and `/metrics`.

## Search quality and limitations

The [November 4 comparison](docs/COMPARISON-2026-11-04.md) covers **older** search profiles and is not a benchmark of v0.30.0. After provider permission changes, the [November 5 Standard runs](docs/ISSUE-RESOLUTION-2026-11-05.md) recorded Hit@5 of **67–69/100**, down from the earlier configuration's 84/100 on the same documentation question set. The newer *offline-only* Clean profile reached **85/254 Hit@10** on SealQA with a 42.7 ms median in its [recorded run](docs/CLEAN-PROFILE.md). These are different datasets and cannot be ranked against each other.

All figures measure whether a reference page was retrieved, **not the accuracy of generated AI answers**. The question sets informed development, some indexes cover relevant documentation sites, and comparisons to stored commercial API results were not simultaneous or held out. These dated experiments are not guaranteed v0.30.0 performance. Do not infer general superiority over Exa or Tavily.

## Documentation

- [Agent installation and verification](docs/AGENT-SETUP.md)
- [Provider terms and supported configurations](docs/PROVIDERS.md)
- [Local crawler and search index](docs/LOCAL-INDEX.md) · [Wikipedia index](docs/WIKIPEDIA-INDEX.md)
- [SearXNG compatibility](docs/COMPATIBILITY.md) · [Historical benchmarks](docs/BENCHMARK.md)
- [Security policy](SECURITY.md) · [Contributing](CONTRIBUTING.md) · [Release procedure](docs/RELEASING.md)

## Contributing and license

Bug reports and contributions are welcome through [Issues](https://github.com/sahenjp/RustXNG/issues), [Discussions](https://github.com/sahenjp/RustXNG/discussions) and pull requests. See [CONTRIBUTING.md](CONTRIBUTING.md), [GOVERNANCE.md](GOVERNANCE.md) and [MAINTAINERS.md](MAINTAINERS.md).

RustXNG is licensed under [AGPL-3.0-or-later](LICENSE). It builds on SearXNG's design and engine behavior; upstream and third-party attribution is preserved in [NOTICE](NOTICE). A code license does not grant permission to collect or redistribute data from external search providers.
