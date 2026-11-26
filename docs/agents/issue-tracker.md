# GitHub issue tracker

Track issues and specs in `rupertgermann/duplicateFinder` with `gh`. Pass `--repo rupertgermann/duplicateFinder` on issue and PR commands to select this repo explicitly.

## Issue operations

- Publish an issue or spec with `gh issue create --title "…" --body-file <file>`.
- Fetch a ticket with `gh issue view <number> --json number,title,body,labels,comments`.
- List tickets with `gh issue list --state open --json number,title,body,labels,comments`. Use `--label` and `--state` to scope the result.
- Link a sub-issue with `gh issue create --parent <parent>` or `gh issue edit <parent> --add-sub-issue <child>`. If GitHub sub-issues are unavailable, add `Part of #<parent>` to the child body.
- Set native dependencies with `gh issue create --blocked-by <blocker>,<blocker>` or `gh issue edit <child> --add-blocked-by <blocker>`. If GitHub dependencies are unavailable, use `Blocked by: #<blocker>, #<blocker>` in the issue body.
- Count open native blockers with `gh api repos/rupertgermann/duplicateFinder/issues/<number> --jq .issue_dependencies_summary.blocked_by`. An issue is unblocked when every blocker is closed.
- Apply or remove a label with `gh issue edit <number> --add-label <label>` or `--remove-label <label>`.
- Comment with `gh issue comment <number> --body-file <file>`. To close an issue, post the explanation first and run `gh issue close <number>`.

## Pull requests as a triage surface

**PRs as a request surface: no.**

When enabled, read the PR and discussion with `gh pr view <number>` and `gh pr view <number> --comments`. Read the patch with `gh pr diff <number>`. Use `gh pr comment`, `gh pr edit --add-label` or `--remove-label`, and `gh pr close` for triage actions. List external open PRs with `gh api --paginate 'repos/rupertgermann/duplicateFinder/pulls?state=open' --jq '.[] | select(.author_association | IN("OWNER","MEMBER","COLLABORATOR") | not)'`. Resolve a bare number with `gh pr view`, then `gh issue view` if needed.

## Wayfinding

- Keep the map in one issue labelled `wayfinder:map`. Link tickets as sub-issues labelled `wayfinder:<type>`, where the type is `research`, `prototype`, `grilling`, or `task`.
- If sub-issues are unavailable, list the children as tasks in the map body and add `Part of #<map>` to each child.
- In map order, take the first open child with no open blockers and no assignee. Claim it with `gh issue edit <number> --add-assignee @me`.
- Resolve the ticket with a comment and close it. Record the decision and ticket link on the map.
