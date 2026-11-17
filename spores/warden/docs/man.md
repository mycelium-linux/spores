# NAME

warden — what every repository in the fleet is missing, and what has rotted

# SYNOPSIS

`warden report [--json] [--all] [--limit N] [--strict] [--fail-on LEVEL] < SNAPSHOT`

`warden explain OWNER/NAME < SNAPSHOT`

`warden ingest [--at RFC3339] < FORGE-PAGES`

`warden checks [--json]`

# DESCRIPTION

A fleet rots in ways nobody notices one repository at a time: a licence that was
never added, a workflow that was written and then broke, a release nobody cut, a
description left empty, documentation whose links stopped resolving, a branch
that has not been touched in a year. Each is small. Together they are the reason
a thousand tokens go into a repository that cannot be released, and the reason
the next person cannot tell which of three hundred repositories is alive.

warden asks nine questions of every repository and answers them from one
document. The questions are a fixed vocabulary (`warden checks` prints it). The
thresholds — how stale is stale, how many branches are too many — are policy,
and the policy travels inside the snapshot so a report can be reproduced from
the bytes alone.

The answer has three values, not two. `pass` means the evidence says yes, `fail`
means it says no, and `unknown` means there was no evidence. Unknown is not a
pass: a repository whose CI state was never collected is not green, it is
unmeasured. `--strict` is a caller saying it would rather see the noise than the
lie.

# TWO COMMANDS, ONE PIPELINE

Collecting and scoring are separate on purpose.

```sh
./collector/collect.sh FuturePresentLabs mycelium-linux > snapshot.json
warden report < snapshot.json
```

Collection needs a token and the network; scoring needs neither. That is why the
same binary runs inside `soil-command`, which links no host capability at all,
and inside a GitHub Action, where the token is the step's own. It is also why
two reports a week apart diff cleanly: nothing in the scoring path could have
changed between them except the fleet.

`warden ingest` is the mapping from GitHub's GraphQL pages to a snapshot, and it
lives here rather than in a `jq` program so that a field the forge renames is a
failing test instead of a silently empty column.

# THE CHECKS

| check | severity | question |
|---|---|---|
| `license` | critical | does the repository declare a licence |
| `ci` | critical | is there a workflow under `.github/workflows` |
| `ci-green` | critical | did the last run on the default branch pass |
| `release` | advisory | has it ever cut a release |
| `description` | advisory | does the forge description say what it is |
| `docs-links` | advisory | do the documentation links resolve |
| `active` | advisory | was it pushed to inside the staleness window |
| `branches` | advisory | are branches under the limit |
| `open-pulls` | advisory | are open pull requests under the limit |

`warden checks` prints the same table with the remedy for each, and `--json`
prints it for a tool.

Three of these ask for evidence a batched collection does not have. `ci-green`
and `docs-links` answer `unknown` until a collector fills `latest_run` and
`links`. `active` answers `unknown` until the snapshot carries `collected_at`.
Filling them is a collector change, not a warden one.

# EXIT STATUS

- `0` — nothing at or above `--fail-on` failed.
- `1` — something did. The default is `critical`, so a missing licence or a red
default branch fails a pull request and a missing description does not.
- `2` — the input could not be read. A bad snapshot is never an empty fleet and
never a green one.

# EXAMPLES

Fail a pull request that adds a repository with no licence:

```sh
gh api graphql --paginate ... | warden ingest > snapshot.json
warden report --fail-on critical < snapshot.json
```

Ask why one repository is red:

```sh
warden explain FuturePresentLabs/bob < snapshot.json
```

Gate on advisories in a nightly job, where a missing description should be
visible but should not page anybody:

```sh
warden report --fail-on advisory --strict < snapshot.json
```

# READING THE REPORT

```
check            fail  unknown   share
license          119        -   ███████████████████       79%
ci                98        -   ████████████              65%
```

`fail` is repositories where the check said no. `unknown` is repositories where
it could not answer. `share` is `fail` over the repositories scored, so a
skipped repository (archived, or a fork) is counted in neither the numerator nor
the denominator.

# EXEMPTIONS

Some repositories do not need a check: a proprietary repository still needs a
question answered, but not a licence file. The snapshot carries that per
repository:

```json
{"org": "fpl", "name": "acme", "exempt": {"license": "proprietary, see legal"}}
```

An exemption is a decision somebody wrote down, so it is reported as `exempt`
with its reason rather than dropped. A check that quietly disappears is a check
the next person cannot audit.

# SEE ALSO

`docs/warden.md`, `collector/fleet.graphql`, `mush doctor` (which checks the
shell, not the fleet).
