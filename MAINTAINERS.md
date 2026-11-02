# Maintainers and access management

## Roster

| Account | Responsibility | Repository access |
| --- | --- | --- |
| [sahenjp](https://github.com/sahenjp) | Owner, release authority, final decisions | Owner/admin |

No additional collaborator invitations have been sent as part of the community setup.
[Apply to maintain an area](https://github.com/sahenjp/RustXNG/issues/new?template=maintainer_application.yml)
or contribute without access through a fork and PR. See [GOVERNANCE.md](GOVERNANCE.md).

## Owner checklist before the first write invitation

1. Agree the area and responsibilities with the candidate; verify their **GitHub** account directly.
   Do not infer it from an X/Twitter display name or a complimentary post.
2. Review their contributions and another person's changes together. Check that they can reproduce tests
   and identify limitations, not just produce a large number of commits.
3. Update this roster and `.github/CODEOWNERS` in a reviewed PR. Include at least two active people able
   to review each other's changes before requiring code-owner approval for every path.
4. Change main protection from zero to **one required approval**, retaining CI checks and stale-approval dismissal.
   Enable code-owner review only after the mapping can support review of the owner's own PRs.
5. Confirm version-tag protection, private vulnerability reporting, and read-only workflow tokens remain enabled.
6. Confirm the candidate is trusted with repository-wide write, including its implications for Actions and release assets.
   Personal-repository write cannot be restricted to one directory; version-tag rules do not make every asset immutable.
   If that trust is not appropriate, keep the role as a public reviewer or consider organization roles separately.
7. Only then send a collaborator invitation. Do not grant admin access as the default next step.
8. After acceptance, verify the actual permission, agree escalation and review availability, and record the decision.

## Owner commands

These are operator examples, **not automatic onboarding**. Replace `GITHUB_LOGIN` only after explicit approval.
The repository is personal-owned; use the collaborator write permission (`push` in the REST API), not an invented
personal-repository `maintain` or `triage` tier.

```bash
# Verify the account and current permission first.
gh api users/GITHUB_LOGIN --jq '{login, html_url}'
gh api repos/sahenjp/RustXNG/collaborators/GITHUB_LOGIN/permission

# Before adding the first write collaborator: require one approval.
gh api --method PATCH repos/sahenjp/RustXNG/branches/main/protection/required_pull_request_reviews \
  -F required_approving_review_count=1

# Send an invitation only for the approved account.
gh api --method PUT repos/sahenjp/RustXNG/collaborators/GITHUB_LOGIN -f permission=push

# Inspect pending invitations and verify accepted access.
gh api repos/sahenjp/RustXNG/invitations
gh api repos/sahenjp/RustXNG/collaborators/GITHUB_LOGIN/permission
```

Cancel an unaccepted invitation through Settings → Collaborators or the invitation API.
Remove accepted access only after an explicit owner decision:

```bash
gh api --method DELETE repos/sahenjp/RustXNG/collaborators/GITHUB_LOGIN
```

Never paste tokens into Issues, PRs, application forms, or shell command arguments.
GitHub authentication is handled by the operator's existing `gh` login.
