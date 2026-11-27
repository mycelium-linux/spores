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
| current installed `acp-agents` module (first audit) | `53e26825f36ff2338fd5c889e8abac93d0055f5d6dae8752201193a773dd1cf5` |
| current installed `acp-agents` lock (first audit) | `37657fb23715012fe1edff1aad450a158f0df4cf2d37adba756ad86f544f24b1` |
| current installed `acp-agents` module (second audit, after another provider build) | `6b1aaae608b9a9d8553c4f508d17955556c7168f35a8bc866835bc048da8545c` |

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
reinstalled runtime: c50ecf50...   (first audit)
reinstalled runtime: 4d515b1c...   (second audit; the provider had moved again)
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

## Staging and rollback

No live install was performed. The scratch roots are disposable; the live
generations are intact:

| Package | Live generation | Previous |
|---|---|---|
| `dev.mycelium.agents` | `5a35eb951166557a05d13c3ca0395392e689632d3c30b1c249c2756ccd08c2c1` | same (only generation) |
| `dev.mycelium.agents-cli` | `350673b6c7e01392023a5b5fa229e8474cb5544229f2e04f0d9be14d3fa6b9e4` | same |
| `dev.mycelium.acp-agents` | `5a9810c162ce38bc0ad88f055ab1166d51a757428c3071453b3e0648fbfe1a3d` | `c96c598a...` |

A live reinstall creates a new generation and sets `previous` to the current
one. The rollback, if the new generation fails after coordination, is:

```sh
loam rollback dev.mycelium.agents --root ~/.local/share/fungos/spores
loam rollback dev.mycelium.agents-cli --root ~/.local/share/fungos/spores
```

The old runtime `85257b2c...` and its `runtime.json` remain in the live
generation directory. Do not delete them. Do not restart MCP; singularity owns
that service, and a generation change becomes visible only when its host
reloads the command.

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

- Native Theseus harness: `./theseus doctor` returns `ok=true`, model
  `deepseek-v4.1-flash`, workspace `~/.local/share/theseus/vfs`.
- Native Theseus records: `theseus:turns` has 74 rows and `theseus:events` has
  1,861 rows at audit time; one turn recorded `THESEUS-IMAGE-OK`. The data is
  real.
- `./theseus events` currently fails: `unknown form or function event/print`
  (the harness CLI event printer is broken). That is a native harness defect,
  separate from the adapter.
- `theseus-agents` adapter: blocked by the `soil:records` link error above.
  `loam run agents` and `loam run agents-core` both fail before list/get/
  dispatch/transcript can be exercised.
- Pi / ACP: `~/.local/share/theseus/vfs/agents/acp.json` is absent, so there is
  no configured ACP endpoint to test. ACP/Pi coverage is **unverified**, not
  passed.
- No fabricated provider coverage: list/get/dispatch/transcript are not
  claimed until the adapter links.

## What would unblock the restore

1. **Loam composer:** walk imports introduced by a composed provider, not only
   the original module's imports. This is the `agents-cli` transitive failure.
2. **Loam grant validation:** accept provider interface namespaces
   (`mycelium:...`) or move provider exports out of `[grants]`; the current
   source rejects the packages before composition.
3. **Records host identity:** make the installed `loam` host and the
   `theseus-agents` module agree on the `soil:records` interface (the WIT source
   matches; the installed binary/module identity does not).
4. **Native harness:** restore `events/print`.
5. **Pi/ACP:** provide `agents/acp.json` with a real endpoint and test it.
6. **Signed deployment:** establish a signed `loam` release path before asking a
   cofounder to install it; `mycelium update` is signed, `loam` is a local build.

## Live restore is blocked

The live MCP deployment is owned by singularity. Do not run the live reinstall
or restart until the Loam owner decides the composer/transitive and
grant/records issues above. The scratch evidence is ready; the live change is
not.
