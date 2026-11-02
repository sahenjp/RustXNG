# Roadmap and contribution areas

Updated 2026-11-02. This records priorities and review gates, not promised delivery dates.
New features remain proposals until code review, current-head CI and the stated evidence are complete.

## First: make the foundation reliable

| Work | Status / gate |
| --- | --- |
| [#23 agent setup and port migration](https://github.com/sahenjp/RustXNG/pull/23) | All six CI checks passed at review time; recommended foundation. No merge performed by the community setup |
| [#12 request/cache identity and deadlines](https://github.com/sahenjp/RustXNG/pull/12) | Rebase intended commits onto current main; prioritize before more authenticated/cached providers |
| [#13 crawler frontier](https://github.com/sahenjp/RustXNG/pull/13) | Rebase and rerun CI; fixes links being lost during completion/restart |
| [#20 number validation](https://github.com/sahenjp/RustXNG/pull/20) | Adopt the targeted fix after current-head CI |
| [#35 resting-engine visibility](https://github.com/sahenjp/RustXNG/pull/35), closes [#17](https://github.com/sahenjp/RustXNG/issues/17) | Adoption candidate: 10 state tests and 3 resting HTTP tests passed locally; still verify current-base CI |
| Published [audit](AUDIT-2026-11-02.md) | Resolve untrusted fetching, URL schemes, size limits and resource controls before expanding fetch-heavy features |

## Community feedback translated into work

| Work | Completion criteria |
| --- | --- |
| [#39 Linux ARM64 distribution](https://github.com/sahenjp/RustXNG/issues/39) | Verified native assets, checksums, startup/search smoke tests and accurate install instructions; distinguish ARM64 from ARMv7 |
| [#40 Japanese errors and broad/fresh retrieval benchmark](https://github.com/sahenjp/RustXNG/issues/40) | Public fixed queries, reference URLs, repetitions, failures, language/time/cache conditions; separate pinpoint and broad-source metrics |
| [#41 agent output budgets](https://github.com/sahenjp/RustXNG/issues/41) | Compact fields, relevant passages, source retention, explicit truncation and well-defined character/byte/token budgets |
| [#30 built-in MCP](https://github.com/sahenjp/RustXNG/issues/30) | Stdio and bounded output, current SDK/MSRV checks, safe page fetching and proxy policy; revise closed #31 rather than treating it as already accepted |
| [#16 Bing market variants](https://github.com/sahenjp/RustXNG/issues/16) | Verify registration cause and compare quality/p95/rate limits before enabling extra default traffic |

Rust implementation, retrieval quality, transfer cost and model context consumption are different dimensions.
See [FAQ](FAQ.md). Keep the existing [SealQA retrieval baseline](SEALQA.md) reproducible;
new workloads must not inherit its scores as a performance guarantee.

## New PR review gates

| PR | Current assessment |
| --- | --- |
| [#32 engine applicability](https://github.com/sahenjp/RustXNG/pull/32) | Useful intent; `cargo check --tests` reproduced 11 errors from the misplaced dictzone test. Fix before adoption |
| [#33 CSE preflight](https://github.com/sahenjp/RustXNG/pull/33) | Preflight and actual request must share one deadline; review token invalidation and concurrent bootstrap before adoption |
| [#34 reference infoboxes](https://github.com/sahenjp/RustXNG/pull/34) | Design decision: preserve the agent `results` path and explicit Wikipedia search, or provide an opt-in display mode and migration. Other engines may still return Wikipedia URLs; this is not a claim all such URLs disappear |
| [#36 language parameter](https://github.com/sahenjp/RustXNG/pull/36) | Important for Japanese clients. Define/test `auto` and `all` in both language and locale paths before merging |
| [#37 canonical reranking](https://github.com/sahenjp/RustXNG/pull/37) | Hold for safe fetching, size/concurrency/deadline limits, canonical cycles/chains and quality evidence. A publisher-supplied canonical is a hint, not verified authority |
| [#38 Qiita](https://github.com/sahenjp/RustXNG/pull/38) | Add after authentication-aware cache identity; verify quota/backoff/default-category behavior and provider compatibility |
| [#24 CiNii](https://github.com/sahenjp/RustXNG/pull/24) | Fix period precision and page arithmetic overflow; retain the Japanese scholarly-source goal |

A blank CI list is not a successful run. Review new contributor workflows before approving their execution;
do not use privileged `pull_request_target` execution of untrusted checkout code as a shortcut.

## Areas seeking participation

Documentation and translations, offline engine fixtures, benchmark reproduction, native ARM64 distribution,
CI, observability and network-fetching safety are useful contribution areas.
Use [CONTRIBUTING.md](../CONTRIBUTING.md), [Discussions](https://github.com/sahenjp/RustXNG/discussions),
or the [maintainer application](https://github.com/sahenjp/RustXNG/issues/new?template=maintainer_application.yml).
Participation is open; privileges require an explicit owner decision under [GOVERNANCE.md](../GOVERNANCE.md).
