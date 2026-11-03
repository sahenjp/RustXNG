# Repository settings record

These JSON files record the settings applied on 2026-11-03. Committing them does **not** automatically apply settings.
Do not run an unreviewed settings file from a contributor branch with an administrator token.

- `repository.json`: Discussions enabled, squash-only merges, automatic deletion of merged branches, auto-merge availability.
- `main-protection.json`: up-to-date required checks from GitHub Actions app 15368, PR workflow,
  conversation resolution, linear history, no force pushes or branch deletion.
- `release-tags.json`: `v*` tag updates/deletion restricted, with only the owner's account as bypass actor. Authorized writers and release automation may create tags; the release pipeline requires matching versions and the exact successful main CI commit before publication.

The main approval count is initially zero because the owner is the only maintainer. Administrator emergency bypass
is retained (`enforce_admins=false`); this is not an unrestricted bypass for ordinary write collaborators.
Before the first write invitation, follow [MAINTAINERS.md](../../MAINTAINERS.md) and require one approval.

Required checks cover Linux/macOS/Windows, no-default-features, msrv, docker, linux-arm64,
workflow-lint, security / audit, docker-arm64 and mysql-integration. PR branches must update to this CI base.

Actions workflow tokens default to read-only. SHA pinning is required at repository level.
`workflow-permissions.json` allows the release bot to create PRs; the GitHub option also permits review approval,
but the workflows do not approve reviews or merge PRs. Bot-created PR workflows require a maintainer's run approval; see [release operations](../../docs/RELEASING.md).
`actions-permissions.json` records the SHA-pinning policy.
Private vulnerability reporting was already enabled and remains enabled. First-time fork workflow approval remains enabled.
Contributor access still follows [MAINTAINERS.md](../../MAINTAINERS.md); repository configuration grants no invitation automatically.

An owner can reconcile repository and main settings after reviewing the live values:

```bash
gh api repos/sahenjp/RustXNG
gh api repos/sahenjp/RustXNG/branches/main/protection
gh api repos/sahenjp/RustXNG/rulesets
# Writes, only after reviewing the JSON:
gh api --method PATCH repos/sahenjp/RustXNG --input .github/repository-settings/repository.json
gh api --method PUT repos/sahenjp/RustXNG/branches/main/protection --input .github/repository-settings/main-protection.json
gh api --method PUT repos/sahenjp/RustXNG/actions/permissions --input .github/repository-settings/actions-permissions.json
gh api --method PUT repos/sahenjp/RustXNG/actions/permissions/workflow --input .github/repository-settings/workflow-permissions.json
```

The tag ruleset already exists (ID 26350142). Review and update that ID rather than creating duplicates:

```bash
gh api repos/sahenjp/RustXNG/rulesets/26350142
gh api --method PUT repos/sahenjp/RustXNG/rulesets/26350142 --input .github/repository-settings/release-tags.json
```

If ownership or the active maintainer roster changes, review the bypass account, CODEOWNERS, and approval count together.
