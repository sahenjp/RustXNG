# Governance and maintainer participation

RustXNG is an open-source project based on SearXNG, licensed AGPL-3.0-or-later.
Participation is open; repository access is granted explicitly, not by popularity, payment, or automated scoring.

## Current responsibility

- Repository owner and release authority: [@sahenjp](https://github.com/sahenjp).
- Additional maintainers: none appointed yet. Contributions and applications are welcome.
- AI tools may assist development and review. The person submitting or merging a change remains responsible
  for understanding it, checking the diff, reproducing relevant tests, and disclosing material limitations.

## Ways to participate

| Role in the project | Work | GitHub access |
| --- | --- | --- |
| Contributor | Reports, documentation, fixtures, benchmarks, code | Forks and PRs; no invitation required |
| Reviewer | Reproduce reports, inspect diffs, suggest tests and priorities | Public reviews; no write access required |
| Area maintainer | Sustain an agreed area, review others' changes, triage work | Explicit collaborator invitation after owner approval |
| Owner | Access management, repository policy, releases and sensitive escalation | Owner/admin; not granted through the application form |

These project roles are not a promise of equivalent GitHub permission names.
This is currently a **personal-account repository**: collaborators have write access; organization-style
Triage/Maintain roles and teams are not available in the same way.
See [GitHub's personal repository permissions](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/repository-access-and-collaboration/permission-levels-for-a-personal-account-repository).
A future organization transfer requires a separate owner decision; none is planned or performed automatically.

## Apply to maintain an area

Use the [maintainer application form](https://github.com/sahenjp/RustXNG/issues/new?template=maintainer_application.yml).
Include your GitHub account, proposed area, relevant contributions or experience, review interests,
and realistic availability. Experience from other projects counts; commit counts are not a ranking system.
Do not put private contact information, credentials, or employment information in a public application.

The owner and existing reviewers discuss the scope publicly where possible. A typical path is a small PR,
a review of another contribution, and an agreed period of shared work before access is considered.
There is no promised response deadline or automatic promotion. Applications can be deferred or declined with a reason.
A write invitation is sent only after the owner has explicitly selected the account and agreed responsibilities.

## Review and merge

- One coherent purpose per PR; large behavior or architecture changes start with an Issue.
- Maintainers first reproduce or understand the issue, then evaluate correctness, compatibility, resource use,
  security, and the evidence. CI passing alone is not proof of correctness.
- Use squash merge. Keep required checks green against the current base and resolve review conversations.
- Do not merge your own substantive change without another person's review once multiple maintainers are active.
- Currently there is one owner, so the required approval count is zero to avoid self-review deadlock.
  The first write collaborator must trigger the onboarding steps in [MAINTAINERS.md](MAINTAINERS.md),
  including an approval requirement and updated CODEOWNERS.
- The owner retains an emergency branch-protection bypass. It is for recovery, not routine merging with failing checks;
  any use should be documented with the reason and follow-up verification.
- Sensitive dependency, authentication, network-fetching, workflow, and release changes need owner review.

## Releases and access

Version tags matching `v*` are protected: only the owner can create, update, or delete them under the active ruleset.
Use a new release tag rather than replacing published binaries. Release authority is a project policy, not a claim that a personal repository can technically scope all write permissions.
A write collaborator may also affect Actions and release assets; tag rules do not protect every existing asset.
Do not grant write access to someone trusted only to triage Issues or review one narrow area.
Access is reviewed when responsibilities change; invitations can be cancelled and access revoked by the owner.
Inactive maintainers can step back without losing attribution for past work.

## Decisions and escalation

Use Issues for reproducible bugs and scoped proposals; use Discussions for usage questions and exploratory ideas.
Record the decision, trade-offs, and evidence on the relevant work item. If agreement is not reached,
the owner makes the final project decision and explains it.
See [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) for conduct and [SECURITY.md](SECURITY.md) for private vulnerability reporting.
