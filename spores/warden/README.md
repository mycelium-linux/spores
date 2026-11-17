# warden

What every repository in the fleet is missing, and what has rotted.

The first real run over `FuturePresentLabs` and `mycelium-linux` — 150 live
repositories, 42 skipped as archived or forks — said this:

```
check            fail  unknown   share
license          119        -   ███████████████████       79%
ci                98        -   ████████████              65%
ci-green           0      150                              0%
release          145        -   ███████████████████████   97%
description       58        -   █████████                 39%
docs-links         0      150                              0%
active            46        -   ███████                   31%
branches           4        -   █                          3%
open-pulls         0        -                              0%
```

79% of repositories have no licence the forge can name, 65% have no workflow,
97% have never cut a release. Four branches exceed fifty: `tailscale` at 1274,
`comfyui-mcp` at 854, `LibreChat` at 254 and `shared-infra` at 183.

## The shape

```text
forge            snapshot            report
(GraphQL) ──ingest──► (repos) ──evaluate──► (findings) ──render──► text | JSON
```

One function in the middle, and it is pure. `evaluate` reads no clock, opens no
socket and touches no path, so the answer a shell prints, the answer CI gates
on, and the answer a view draws are the same answer from the same bytes. That
is the whole reason a check is written once instead of twice.

## Where things live

```text
src/check.rs     the vocabulary: nine questions and their severities
src/snapshot.rs  the fact sheet a collector produces
src/policy.rs    the thresholds, and evaluate()
src/report.rs    the verdicts, the rollup, and the terminal rendering
src/ingest.rs    GitHub GraphQL pages -> snapshot (tested, not a jq program)
src/date.rs      RFC 3339 in, whole days out
src/bin/warden.rs the `wasi:cli` command
collector/       the reference collector: one query and one shell script
tests/           the document contract, from forge bytes to report JSON
```

## Build

```sh
cargo test --features cli                       # the evaluator and the documents
cargo build --release --target wasm32-wasip2 \
  --features cli --bin warden
cp target/wasm32-wasip2/release/warden.wasm bin/warden.wasm
```

The package ships `bin/warden.wasm`, and `spore.toml` declares it both as the
package's `[module]` and as its one `[[commands]]` entry — the shell finds it
there, and there is no second package to install.

## Run it

```sh
./collector/collect.sh FuturePresentLabs mycelium-linux > snapshot.json
warden report < snapshot.json
```

Inside `mush`, where `soil-command` is the PATH and no command may reach the
network, the same two steps work because the first one happened somewhere else:

```sh
cat snapshot.json | warden report
```

## Decisions worth knowing

**Unknown is not a pass.** Every check answers `pass`, `fail`, `unknown` or
`exempt`. A repository whose CI state was never collected is unmeasured, not
green, and the summary counts it separately. `--strict` is a caller saying it
would rather see the noise than the lie.

**The collector fetches; warden scores.** `soil-command` links exactly one
capability (`soil:records`) and refuses a package by name for anything else, so
a command that could fetch would be a command that does not run in `mush`. It is
also the better shape: two reports a week apart diff cleanly because nothing in
the scoring path could have changed between them except the fleet.

**The mapping is code, not `jq`.** `ingest.rs` turns GraphQL pages into a
snapshot, with a test per shape the forge returns. Forty fields of `jq` is a
program nobody reviews and nobody tests, and a renamed field there is a silently
empty column rather than a failing test.

**Policy travels in the document.** Thresholds and per-repository exemptions
are inside the snapshot, so a report is reproducible from its bytes alone.
An exemption is reported as `exempt` with its reason — a check that quietly
disappears is one the next person cannot audit.

**Fifty repositories per page, not a hundred.** A hundred-repository page gets
answered with a 502 by the proxy in front of the API often enough to matter, and
two extra round trips over the fleet cost less than a collection that fails at
random.

## What is not here

- **A view.** The report JSON is the document a GPUI Spore draws; that package
  is `warden-9bum` in Marbles and needs a decision about where the snapshot is
  served from.
- **A cached report.** `soil:records` is where a report belongs so `mush` can
  answer without a pipe, and that needs a writer (`warden-2d4h`).
- **`ci-green` and `docs-links` evidence.** Both checks exist and answer
  `unknown`; filling them is a collector change (`latest_run`, `links`), not a
  warden one.
