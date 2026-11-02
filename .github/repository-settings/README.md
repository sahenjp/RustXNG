# Repository settings record

These JSON files record the settings applied on 2026-11-02. Committing them does **not** automatically apply settings.
Do not run an unreviewed settings file from a contributor branch with an administrator token.

- `repository.json`: Discussions enabled, squash-only merges, automatic deletion of merged branches, auto-merge availability.
- `main-protection.json`: up-to-date required checks from GitHub Actions app 15368, PR workflow,
  conversation resolution, linear history, no force pushes or branch deletion.
- `release-tags.json`: `v*` tag creation/update/deletion restricted, with only the owner's account as bypass actor.

The main approval count is initially zero because the owner is the only maintainer. Administrator emergency bypass
is retained (`enforce_admins=false`); this is not an unrestricted bypass for ordinary write collaborators.
Before the first write invitation, follow [MAINTAINERS.md](../../MAINTAINERS.md) and require one approval.

Required check names match PR #23: Linux/macOS/Windows, no-default-features, msrv, docker.
Main does not yet contain that new workflow until #23 merges. Other PRs need to update to that base to produce all checks.

Actions workflow tokens were verified read-only, with automated PR approval disabled.
Private vulnerability reporting was already enabled and remains enabled. First-time fork workflow approval remains enabled.
No collaborator invitation, organization transfer, PR merge, or release was performed by this setup.

An owner can reconcile repository and main settings after reviewing the live values:

```bash
gh api repos/sahenjp/RustXNG
gh api repos/sahenjp/RustXNG/branches/main/protection
gh api repos/sahenjp/RustXNG/rulesets
# Writes, only after reviewing the JSON:
gh api --method PATCH repos/sahenjp/RustXNG --input .github/repository-settings/repository.json
gh api --method PUT repos/sahenjp/RustXNG/branches/main/protection --input .github/repository-settings/main-protection.json
```

The tag ruleset already exists (ID 26350142). Review and update that ID rather than creating duplicates:

```bash
gh api repos/sahenjp/RustXNG/rulesets/26350142
gh api --method PUT repos/sahenjp/RustXNG/rulesets/26350142 --input .github/repository-settings/release-tags.json
```

If ownership or the active maintainer roster changes, review the bypass account, CODEOWNERS, and approval count together.
