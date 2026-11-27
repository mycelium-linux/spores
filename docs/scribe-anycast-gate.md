# Scribe anycast UI gate — evidence and blocker

Status: **initial gate partially met; not accepted as multi-node anycast.** Read-only
inspection and a direct endpoint test; no production profile, grant, or service change.

## Evidence

- Neo: `https://scribe.myc/` returns **200**, body title **Scribe**, 47,784 bytes.
  This is the actual Scribe UI, not `/healthz`.
- Neo: `scribe-ui` process is listening on `0.0.0.0:8088`; `scribe` ingest process
  is on `0.0.0.0:8096` (WSS).
- Neo owner lease: `kind=scribe`, `names=[scribe]`, endpoint
  `http://192.168.10.82:8088`, fresh at the observed time.
- Neo frontdoor: route `scribe` with hosts `scribe.fpl.dev`, `scribe.myc`, upstream
  `http://127.0.0.1:8088`, `service_kind=scribe`.
- Studio and Agora can read the actual UI and capture metadata directly from the
  Neo endpoint:
  - `curl -H 'Host: scribe.myc' http://192.168.10.82:8088/` → 200, 47,784 bytes
  - `/api/captures` → 200, 50 metadata rows (no private transcript content inspected)
- Studio `https://scribe.myc/` → 404 “No app at this name”; Agora same via its local
  frontdoor. The local DNS name resolves to each node's own frontdoor, not to Neo.

## Root cause

The frontdoor strips the inbound `Host` header (`server.rs:skip_header`) and
upstreams see the URL authority (for example `100.120.101.5:8088`). The Scribe
web surface only accepts its declared host (`scribe.myc`/`scribe.local`); a
direct `Host: 100.120.101.5` or `Host: 192.168.10.82` returns `unknown host`.
A local route to the remote endpoint therefore cannot serve the UI without a
host-preserving route or an application host allowlist change.

The Neo route works only because its upstream is loopback (`127.0.0.1:8088`),
which the Scribe gateway accepts. The owner lease observer (`43d5547b…`) also
does not match the current node ID (`3b993f12…`), so the service catalog does not
project `scribe` and the frontdoor cannot derive a cross-node route.

## Why no fix was applied

- Restarting the Scribe UI or frontdoor would interrupt the current path.
- Adding a local frontdoor upstream to `100.120.101.5:8088` produced a 404
  `unknown host`; the route was reverted to the pre-change backup.
- A host-preserving frontdoor option or an app host allowlist change is a code
  change, not a route-config change; it needs review/build/coordinated rollout.
- No second local VFS/records replica exists on Studio/Agora.

## Offline / origin-loss

Not tested. There is one live origin (Neo) and no replica on Studio/Agora; stopping
Scribe or Neo would be an unjustified interruption. The correct failure behavior
is 502/offline, but it must be demonstrated with a controlled canary origin, not
by taking the live origin down.

## Required next slice

1. Add a route-level `preserve_host` (or equivalent) for configured remote
   upstreams, or make Scribe's declared host list include the upstream authority.
2. Correct the owner-lease observer/node identity so `mycelium services` projects
   `scribe`; then native derived routes and DNS can reach the leader.
3. Add a second authorized node with a VFS/records replica or an explicit remote
   route; verify UI + authorized capture read, not just health.
4. Only then test controlled origin loss and cross-Desktop denial with separate
   VFS/records roots.
