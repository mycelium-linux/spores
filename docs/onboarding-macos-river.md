# macOS onboarding: River

Date: 2026-11-25. Verified on `Averys-Mac-Studio.local` and
`Averys-MacBook-Neo.local`. These are the commands that actually ran, not a
proposed rewrite.

## What is already installed

| Tool | Studio | Neo |
|---|---|---|
| `loam` | `/Users/ajmwagar/.cargo/bin/loam` | same |
| `loamd` | `/Users/ajmwagar/.cargo/bin/loamd` | same |
| `mycelium` | `/Users/ajmwagar/.local/bin/mycelium` | same |
| `mycelium-frontdoor` | `/Users/ajmwagar/.local/bin/mycelium-frontdoor` | same |
| `fpl` | `/Users/ajmwagar/.cargo/bin/fpl` | same |
| `iroh-tun` | `/Users/ajmwagar/.local/bin/iroh-tun` | same |
| `tend` | `/Users/ajmwagar/.local/bin/tend` | same |
| `soil-lang` | **missing on PATH** | `/Users/ajmwagar/.local/bin/soil-lang` |
| `mmctl` | **missing** | **missing** |

`loam status` on studio lists 13 installs, including the agent stack:
`dev.mycelium.agents`, `dev.mycelium.acp-agents`,
`dev.mycelium.theseus-agents`, `dev.mycelium.agents-cli`,
`dev.mycelium.acp-agents-cli`.

On neo, `loam status` lists only `dev.fungos.composio`; neo carries symlinked
Spores (`colony`, `scribe`, `terminal`, `weeper`, `xplorer`, …) without the same
managed-install state.

## 1. SSO

Verified endpoints:

```sh
curl -fsS https://sso.fpl.dev/.well-known/openid-configuration
# issuer=https://sso.fpl.dev, authorization_endpoint=.../authorize, token_endpoint=.../token
```

Enroll / refresh this Mac:

```sh
fpl login --bootstrap
fpl whoami --json
fpl doctor
```

Observed:

- Studio: device credential present, `session: expired_or_revoked`. Re-run
  `fpl login --bootstrap` locally.
- Neo over SSH: `device_credential: unavailable`,
  `session: credential_store_unavailable`. The keyring is not available to a
  non-GUI SSH session; log in at the console or use the device-code flow.
- Studio `fpl doctor`: 5 passed, 4 failed — missing `bd`, missing `.beads`,
  missing project `AGENTS.md`/`CLAUDE.md` in `~/src/spores`. A fresh clone needs
  `fpl init` / Beads before doctor is green.

## 2. Mycelium

```sh
mycelium daemon status
mycelium doctor --json
mycelium frontdoor status --json
```

Verified:

- Studio: daemon `v0.1.3`, frontdoor listening on `127.0.0.1:8443`, ingress on
  `127.0.0.1:443` and `:80`, 34 routes, certificate trusted. `frontdoor auth`
  warns `auth_required=false` — safe only while loopback-only.
- Neo: daemon `v0.1.3`, frontdoor listening, 6 routes, certificate trusted for
  `ai.myc`; `mycelium doctor` returns `ok=false` because `iroh-tun publish` and
  `iroh-tun tunnel` fail.
- Both: running binary is a local build; automatic updates disabled; Gatekeeper
  notarization warning.

## 3. iroh-tun

```sh
iroh-tun --version       # iroh-tun 0.1.0
iroh-tun --help
```

Verified flags: `--tun`, `--mycelium`, `--status-file`, `--secret`,
`--print-id`, `--route`, `--peer`, `--listen`, `--refresh-secs`.

Neo is the failing case:

```text
iroh-tun publish: the catalog has no fresh iroh-tun observation for this node
iroh-tun tunnel: utun220 is alive but not reporting; external iroh-tun has not
  written a fresh status; log: /Users/ajmwagar/.mycelium/iroh-tun.log
```

The binary is installed, but the privileged tunnel service is not publishing.
Check the log and the launchd/privileged service before treating the overlay as
up. `mycelium doctor` names this as a gap, not a warning you can ignore.

## 4. Loam and Spores

```sh
loam status
loam install <spore-dir> \
  --root ~/.local/share/fungos/spores \
  --provider-root ~/.local/share/fungos/spores \
  --allow-unsigned-dev
```

Spore roots on studio:

```text
~/.local/share/fungos/spores          installed packages
~/.mycelium/spores                    personal Spores
~/src/fungOS/spores/spores            source packages
```

The agent stack is source-installed in the first root. `agents` is a
build-time composition over `theseus-agents` and `acp-agents`; reinstall the
consumer after a provider change. See
[`agents-composition-regression.md`](agents-composition-regression.md).

## 5. Mattermost

```sh
curl -fsS 'https://mm.fpl.dev/api/v4/config/client?format=old'
# SiteName=Mattermore, SiteURL=https://mm.fpl.dev
```

`mmctl` is not installed on either Mac. Plugin install/configuration and bot
creation require Mattermost System Admin access on the server, not a macOS CLI.
The bot token is not in a checked-in file; locate it in the password manager or
mint a fresh bot token.

## 6. Launchd checks

```sh
launchctl list | grep -E 'mycelium|loam|iroh|frontdoor|records'
```

Observed on studio:

- `dev.fpl.mycelium-frontdoor` running
- `dev.fpl.mycelium-ingress-https` / `http` running
- `dev.fungos.records-iroh` running
- `dev.mycelium.loamd` last exit `-15` (not running at the time of the audit)

Do not restart `dev.fpl.mycelium-mcp-backend` or `dev.fpl.mycelium-mcp-oauth`;
singularity owns the live MCP deployment.

## Exact first-hour checklist for River

Run in this order, on the Mac console (not a headless SSH session):

```sh
# 1. identity
fpl login --bootstrap
fpl whoami --json
fpl doctor

# 2. network control plane
mycelium daemon status
mycelium doctor --json
mycelium frontdoor status --json

# 3. overlay
iroh-tun --version
# inspect the tunnel service and /Users/ajmwagar/.mycelium/iroh-tun.log
# if it is not publishing, fix that before testing mesh access

# 4. runtime and packages
loam status
loam install ~/src/fungOS/spores/spores/agents \
  --root ~/.local/share/fungos/spores \
  --provider-root ~/.local/share/fungos/spores \
  --allow-unsigned-dev

# 5. Mattermost reachability
curl -fsS 'https://mm.fpl.dev/api/v4/config/client?format=old'
```

## Gaps

1. **`fpl` session expired on studio**, and neo's keyring is unavailable over
   SSH. Re-enroll on the console.
2. **`iroh-tun` is not publishing on neo**; `utun220` is alive but stale. The
   overlay is not trustworthy until the status file is fresh.
3. **`loamd` source build spins at startup**; the stable daemon is the previous
   binary. Pin a known-good binary on a new Mac.
4. **`agents` aggregator is stale on studio**; reinstall the consumer after the
   provider change.
5. **`mmctl` and Mattermost admin access are missing**; bot tokens/plugin
   config are a server-side gap.
6. **`soil-lang` is missing on studio PATH**; Theseus/Soil source work needs it.
7. **`fpl doctor` fails on missing `bd` / `.beads` / project agent files**; run
   `fpl init` or clone a bootstrapped repo.
8. **Mycelium is a local build with automatic updates disabled**; decide whether
   to adopt a signed release before onboarding more machines.
9. **Gatekeeper notarization warning** on the Mycelium binary; release builds
   should be signed and notarized.

## Signed distribution and day-one Spores

Do not build a second installer. Use the existing surfaces:

```sh
# signed Mycelium update path (the doctor reports when the running binary is a
# local build; this is the intended release path)
mycelium update apply --write

# packaged / claimed setup
mycelium setup --packaged --claim-file PATH
mycelium setup --repair --site SITE

# identity and repository bootstrap
fpl login --bootstrap
fpl init
```

`fpl install <artifact>` is the manifest-driven install path, but artifact
hosting is still a known gap (`fpl.dev/releases/latest.json`); do not invent a
parallel downloader. `loam` is a local build on both Macs and is not notarized;
there is no signed/notarized `loam` artifact verified here.

Day-one Spore dependencies for a new Mac:

| Need | Status |
|---|---|
| `loam` + `loamd` | present, local build |
| `theseus` harness | source at `~/src/singularity`; needs `soil-lang` |
| `agents` / `agents-cli` | installed, but transitive composition and grant/records blockers |
| `theseus-agents` | installed, blocked on `soil:records` link |
| `acp-agents` | installed, no Pi/ACP endpoint configured |
| `colony`, `scribe`, `terminal`, `weeper`, `xplorer` | studio links present |
| `records-iroh` | launchd service present on studio; verify on neo |
| `iroh-tun` | binary present; neo tunnel not publishing |

The onboarding dependency graph is not complete until the agents stack links.
Run `tools/check-agents-composition.sh` and `tools/check-agents-transitive.sh`
on the new host before calling the Spore set ready.
