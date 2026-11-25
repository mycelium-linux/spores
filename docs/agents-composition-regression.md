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
