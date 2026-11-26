# Agents aggregator: stale composition regression

Date: 2026-11-25. Host: `Averys-Mac-Studio.local`.

## Observed

The installed `agents` aggregator is a build-time composition. Its live
`runtime.json` links an older `acp-agents` module than the one currently
installed:

| Item | Value |
|---|---|
| live `agents` runtime | `85257b2cc77bf036da1d3aab8bc25217d0af88bd20570eaa0de893b0659d05ec` |
| runtime.json links `acp-agents` module | `4579af5032b9b2fac9c864cf9a70f4c94b67c2649d40f6bf678902dc857da482` |
| runtime.json links `acp-agents` lock | `59bb4acb10cfab6e0bd9a5f4889aed61e24bceae6883bcf3e9e71c690ab43048` |
| current installed `acp-agents` module | `53e26825f36ff2338fd5c889e8abac93d0055f5d6dae8752201193a773dd1cf5` |
| current installed `acp-agents` lock | `37657fb23715012fe1edff1aad450a158f0df4cf2d37adba756ad86f544f24b1` |

Both packages are version `0.1.0`. The provider changed without a version bump;
the consumer was not reinstalled, so its generation and host link stayed on the
old bytes.

## Repro / proof

`tools/check-agents-composition.sh` never writes the live root. It copies the
live composition cache into a scratch root, installs the checked-in `agents`
package there against the live providers, and compares every `runtime.json`
provider link to the current installed provider bytes.

```sh
cd ~/src/fungOS/spores
./tools/check-agents-composition.sh
STRICT=1 ./tools/check-agents-composition.sh   # host audit: fails if live is stale
```

Observed:

```text
live agents runtime: 85257b2c...
RESULT: live aggregator is stale:
  dev.mycelium.acp-agents 4579af... 53e268...
reinstalled runtime: c50ecf50...
PASS: cache-preserving reinstall recomposed every provider link
```

The scratch reinstall used the live `.loam/compositions` cache (copied, not
bypassed) and still recomposed to the current provider bytes. The cache is
content-addressed by module bytes; the stale result is the live consumer not
being reinstalled, not a cache-key bug.

## Fix

Reinstall the consumer after its provider changes, in the live root:

```sh
loam install ~/src/fungOS/spores/spores/agents \
  --root ~/.local/share/fungos/spores \
  --provider-root ~/.local/share/fungos/spores \
  --allow-unsigned-dev
```

Do **not** do this on a host whose live MCP deployment depends on the
aggregator without coordinating the Loam owner and `singularity`. The live MCP
service is owned by singularity; this document does not restart or redeploy it.

The expected post-reinstall state is the scratch state above: `runtime.wasm`
`c50ecf50...`, `runtime.json` linking `acp-agents` `53e268...` / lock `37657...`.

## Regression rule

A provider change with no version change is not visible to a consumer until the
consumer is reinstalled. The check script is the regression gate:

- default mode proves a reinstall fixes a stale live link;
- `STRICT=1` fails the host audit while the live consumer is stale.

Add this to the provider release checklist: reinstall every consumer that
composes the provider, then run `STRICT=1` on the host.

## Scope / non-goals

- No Loam source change is proposed here. The cache and composition key behaved
  correctly under a cache-preserving reinstall.
- No live install, reload, or MCP restart was performed.
- The `agents` and `acp-agents` versions remain `0.1.0`; this is a composition
  and host-link refresh, not a semantic version change.

## Second blocker: transitive composition (`agents-cli`)

`agents-cli` imports only `mycelium:agents/cli`. Its provider, `agents`, imports
`theseus-agents` and `acp-agents`. A scratch install of `agents-cli` produced a
`runtime.json` that contains only the direct provider:

```text
dev.mycelium.agents
```

`tools/check-agents-transitive.sh` is the regression:

```sh
./tools/check-agents-transitive.sh
# TRANSITIVE-MISSING: dev.mycelium.theseus-agents dev.mycelium.acp-agents
# exit 1
```

This is a Loam composer issue, not a cache issue: the `compose` loop iterates
the imports of the original module, then composes a provider and advances
`current`. Imports introduced by that provider are not walked. The direct
`agents` package has theseus/acp as direct imports, so its own composition is
fine; the wrapper is not.

The host also fails at runtime on the live wrapper:

```text
$ loam run agents
loam: component imports instance `mycelium:theseus-agents/observe@0.1.0`,
but a matching implementation was not found in the linker:
instance export `agents` has the wrong type: function implementation is missing
```

## Third blocker: provider grant namespace and records identity

The current `loam` source build rejects the packages before composition:

```text
soil: agents: invalid spore: grant "mycelium:acp-agents" is not a
  soil:/wasi:/fungos:/loam: capability
```

The `mycelium:` names are provider exports/imports, not host capabilities. The
installed `loam` binary accepts them (it installed the packages), but then
fails to link the native Theseus adapter:

```text
$ loam run agents-core
loam: component imports instance `soil:records/records@0.1.0`,
but a matching implementation was not found in the linker:
instance export `scan` has the wrong type: function implementation is missing
```

The `soil:records` WIT files in `theseus-agents/wit/deps` and `loam/wit` are
currently identical, so this is an identity/version question between the
installed `loam` host and the adapter module, not a source diff we can paper
over. Do not change the grant or records contract without the Loam owner.

## Native Theseus and Pi coverage

- Native Theseus harness: `./theseus director --bot ...` loads and runs; the
  final reply was empty in this environment, so it is not evidence of a working
  model turn.
- `theseus-agents` adapter: blocked by the `soil:records` link error above.
- Pi / ACP: `~/.local/share/theseus/vfs/agents/acp.json` is absent, so there is
  no configured ACP endpoint to test. The ACP adapter coverage is **unverified**,
  not passed.
- `agents-core` list/get/dispatch/transcript cannot be claimed until the
  adapter links. No fabricated provider coverage.

## Live restore is blocked

The live MCP deployment is owned by singularity. Do not run the live reinstall
or restart until the Loam owner decides the composer/transitive and
grant/records issues above. The scratch evidence is ready; the live change is
not.
