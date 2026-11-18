# fleet-health

The observation half of the fleet feed. Each host runs `fpl-node-health` on a
timer, writes one event per check, and `fpl-node-health-publish` drains those
events onto Unibus as `fpl.node.health.v1`. The consumer (elmers
`notifications/fleet`) posts one nominal line and the failing subjects.

```text
timer ─▶ fpl-node-health ─▶ spool ─▶ fpl-node-health-publish ─▶ hub ─▶ elmers ─▶ Mattermost #fleet
              └─ journal, events.jsonl, Prometheus textfile
```

## Checks

| stream | what it means |
|---|---|
| `mesh` | `mycelium topology` readable; every expected node present |
| `probe` | the public URLs answer |
| `connector` | `cloudflared-edge.service` active |
| `shroud` | `GET :7789/healthz` answers `{"ok":true}` (7789 is firewalled; the check tunnels over ssh) |
| `bob` | the build registry's `/pressure`: zero healthy builders is red |

`NODE_HEALTH_STREAMS` selects what a host reports. `NODE_HEALTH_SHROUD` and
`NODE_HEALTH_BOB` name endpoints as `name=url` pairs (`ssh://` is accepted for
shroud). A frame is published only when a state *changes*, so a stable fleet is
silent.

## Install

The scripts are host tools, not a service in this repo. Copy them to
`/usr/local/bin` (Linux) or `~/.local/bin` (macOS) and run the cycle from the
host's scheduler:

- Linux: `fpl-node-health.service` (`Type=oneshot`,
  `ExecStartPost=.../fpl-node-health-publish`) plus an
  `OnUnitActiveSec=60s` timer.
- macOS: a launchd job with `StartInterval 60` pointing at the cycle wrapper.

The publisher needs a producer token for the hub
(`/etc/unibus/tokens/node-health.token` on Linux) and the hub's installation id.
`fpl-node-health-cycle.example` shows the environment the cycle expects.

## Why this lives here for now

The end state is a Spore that emits these checks through `soil:metrics`
(`wit/soil-metrics.wit` in the soil repo) and publishes alerts with
`soil:unibus`, so one host binary serves `/metrics` and every alert is a rule
rather than a script. Until that wiring lands, this is the honest interim: one
script, one publisher, one kind.
