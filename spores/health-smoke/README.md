# Health smoke

One 200 and one counter. A probe target, not a product: it exists so the
runner's two claims are testable from outside a unit test — a `wasi:http`
component that imports `soil:metrics` instantiates at all, and `GET /metrics`
renders the registry that guest wrote.

```console
$ soil --root /tmp/smoke spore run spores/health-smoke --listen 127.0.0.1:39311 --state /tmp/smoke/state
$ curl -H 'Host: health-smoke.local' 127.0.0.1:39311/healthz
ok
$ curl -H 'Host: health-smoke.local' 127.0.0.1:39311/readyz
$ curl -H 'Host: health-smoke.local' 127.0.0.1:39311/
ok
$ curl -H 'Host: health-smoke.local' 127.0.0.1:39311/metrics
# HELP health_smoke_answered_total Requests health-smoke answered with 200
# TYPE health_smoke_answered_total counter
health_smoke_answered_total 2
```

Three facts in that transcript:

- `/healthz`, `/readyz` and `/metrics` are the **host's** and never reach the
guest, which is why the counter reads 2 after two requests to `/` and not 5.
- `/readyz` is empty because this Spore was granted no dependency that has a
readiness state: an empty check list is ready, and the body says nothing rather
than naming a dependency it does not have.
- Remove `"soil:metrics"` from `[grants]` and the Spore still answers 200 while
`/metrics` stays empty. A refused write is `denied`, and a metric never fails
the work it describes.

## Building

The module is built for `wasm32-wasip2` and committed, because a package carries
its bytes:

```console
$ cd spores/health-smoke && cargo build --release --target wasm32-wasip2
$ cp target/wasm32-wasip2/release/health_smoke.wasm module.wasm
```

`wit/deps/http.wit` is `wasi:http@0.2.12` trimmed to `types` and
`incoming-handler` — see the header in that file for why the outgoing half is
not vendored.
