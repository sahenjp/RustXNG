# Community setup record — 2026-11-02

## Live settings applied and read back

- Description now describes metasearch/grounding without claiming complete SSRF hardening.
- Discussions enabled; Q&A, Ideas, General and other default categories are available.
- Squash merge only; merged-branch deletion and auto-merge availability enabled.
- Main: PR workflow, up-to-date six GitHub Actions checks, resolved conversations, linear history;
  force pushes and deletion disabled. Approval count remains zero for the current single-owner roster.
- Owner emergency bypass retained. It must not be presented as mandatory CI enforcement against the owner.
- Tag ruleset 26350142 protects creation/update/deletion of `v*`, with only sahenjp as bypass actor.
- Workflow tokens were already read-only and automated PR approval disabled. Private vulnerability reporting was
  already enabled. Those settings remain intact. First-time contributor workflow approval remains enabled.
- No collaborator invitation, organization transfer, PR merge, or release was performed.

Versioned desired values and operator instructions are in [.github/repository-settings](../.github/repository-settings/README.md).
Personal-repository Write is broad: tag rules do not restrict every release asset or Actions operation.
The [onboarding checklist](../MAINTAINERS.md) records that limitation and the approval change needed before a write invitation.

## Participation and triage

Added governance, a real maintainer roster, CODEOWNERS routing, an application form and a benchmark form.
Updated contribution/review guidance, private security reporting information and conduct-contact instructions.
A dedicated private conduct mailbox is not yet configured; the contact procedure states that limitation explicitly.

Existing bug/feature Issue forms contained unquoted colon/backtick scalar values that were invalid YAML;
these were quoted and all forms were parsed and structurally checked locally.
Markdown, local links, bilingual README heading structure and the settings JSON were validated.

New work items:

- [#39 verified Linux ARM64 distribution](https://github.com/sahenjp/RustXNG/issues/39)
- [#40 Japanese errors and broad/fresh search benchmarks](https://github.com/sahenjp/RustXNG/issues/40)
- [#41 compact output and explicit context budgets](https://github.com/sahenjp/RustXNG/issues/41)

New PRs #32–#38 were reviewed with local Claude Code assistance. Findings are in the [roadmap](ROADMAP.md).
The #32 build failure was independently reproduced; #35 state tests (10) and resting HTTP tests (3) passed locally.
No external provider performance figures in those PRs were independently remeasured during this community setup.
`needs-ci`/`needs-design` and area labels were added without submitting public review verdicts or changing PR states.

## Merge dependency

This community branch is based on PR #23 and targets that branch so the community diff can be reviewed separately.
Merge #23 into main first; then retarget/rebase this PR onto main and rerun its checks before merging.
The setup does not merge #23 or any other PR automatically.
