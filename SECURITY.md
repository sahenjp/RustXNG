# Security

## Private reporting

Please report vulnerabilities through [GitHub private vulnerability reporting](https://github.com/sahenjp/RustXNG/security/advisories/new),
not a public Issue or Discussion. This feature is enabled for the repository.
Include the affected version, configuration, minimal reproduction and expected impact, without real credentials or private data.
The owner coordinates investigation and disclosure; no response-time guarantee is currently offered.

## Current limitations

RustXNG retrieves untrusted upstream information. Local hosting and a successful health check do not establish safety.
The [2026-11-02 audit](docs/AUDIT-2026-11-02.md) records findings against commit fef998b, including DNS/IPv6/redirect
SSRF gaps, unsafe result URL schemes, response-size limits, cache identity and research API resource limits.
Read that report and the current fixes before exposing the service or enabling untrusted page fetching.

Bind local-agent services to loopback; publish container ports with an explicit host IP.
The engine timeout and suspension mechanisms are availability controls, not complete SSRF or memory-exhaustion defenses.
An authenticated proxy is not automatically a safe resolver for untrusted destinations.

## Dependency checks

```bash
cargo install cargo-audit --locked
cargo audit
```

The recorded audit found `ctutils 0.4.2` / RUSTSEC-2026-0385 and unsoundness advisories for `lru 0.12.5`,
plus maintenance warnings. The default `db-engines` feature enables the PostgreSQL and MySQL dependency chains;
it is incorrect to claim these dependencies are absent from the default build.
See the [recorded advisory output](docs/audit/2026-11-02/dependencies.json).
This is a dated snapshot, not a claim about the latest advisory database or exploitability of every dependency path.

The [2026-11-03 audit comparison](docs/audit/2026-11-03/dependencies-summary.json) records an update to
ctutils 0.4.4 for RUSTSEC-2026-0385. The vulnerability section is then empty, but two `lru 0.12.5`
unsoundness advisories through MySQL and three maintenance warnings remain. They are tracked in
[issue #64](https://github.com/sahenjp/RustXNG/issues/64); this is not an advisory-free dependency tree.

Updates require review of actual usage, a locked dependency check, tests and platform/MSRV verification.
Do not silently suppress a security advisory or present a clean normal CI run as a security audit.

## Maintainer and workflow access

Workflow tokens are read-only by default. Release automation requests PR-creation and publication permissions only in its trusted jobs; it does not approve reviews or merge release PRs.
Version tags are protected against updates and deletion; publication requires the exact versioned commit to have passed main CI. New collaborators must follow the review and access procedure in [MAINTAINERS.md](MAINTAINERS.md).
No application, PR, social-media reply, or AI recommendation automatically grants write or administrative access.

## Continuous advisory checks

The CI and daily security workflow record the complete RustSec report. Vulnerabilities always fail.
Known unsoundness and maintenance findings are listed in `.github/security-baseline.json`, tied to issue #64,
and remain visible in logs and artifacts. New findings, changed affected versions, invalid reports and an expired
baseline fail the check. The current baseline expires on 2026-12-03; it is not a statement that the dependencies are safe.
