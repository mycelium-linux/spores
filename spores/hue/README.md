# hue

A CLI Spore for Philips Hue. It owns Hue semantics — targets, scenes, and the
sRGB-to-CIE-xy conversion the bridge expects — and nothing else. The host owns
the connection and the origin allowlist.

```sh
hue --bridge http://192.168.1.10 pair
hue --bridge http://192.168.1.10 --credential geode://home/hue/bridge status
hue --bridge http://192.168.1.10 --credential geode://home/hue/bridge on
hue --bridge http://192.168.1.10 --credential geode://home/hue/bridge --light 3 color '#ff8800'
hue --bridge http://192.168.1.10 --credential geode://home/hue/bridge --group 1 scene Relax
```

`--json` prints the bridge's reply instead of the one-line summary, which is
what an agent wants when it needs to inspect state. `pair` with `--json` is
intended for the one-time key-capture step.

## Pairing

Press the physical link button on the bridge, then run:

```sh
hue --bridge http://192.168.1.10 --json pair | jq -r '.[0].success.username'
```

The bridge returns the application key once. Store it in Geode, never in the
spore or a record:

```sh
printf %s "$HUE_KEY" | geoded --store ~/.geode/store.db \
  --key-file ~/.geode/store.key \
  --put geode://home/hue/bridge --purpose hue.control
```

`geoded` loads the store at startup. Restart it after `--put` so the live
broker sees the new entry; the socket lease is read-only by design.

## Capability

Only `soil:http`, and only for origins the host was told to allow. The key
stays in Geode; the host leases it and substitutes it into the v1 URL path. The
Spore only sees the opaque `geode://home/hue/bridge` handle.

```sh
soil-http --allow http://192.168.1.10:80 \
  --geode-socket ~/.geode/geode.sock --geode-purpose hue.control \
  spores/hue/module.wasm \
  --bridge http://192.168.1.10 --credential geode://home/hue/bridge status
```

`--key` remains for native tests and one-off debugging. It is not the shipping
path: a v1 key in an argument is visible to peers on the same host.

## Build

```sh
cargo build --target wasm32-wasip2 --release -p spore-hue
cp target/wasm32-wasip2/release/spore_hue.wasm spores/hue/module.wasm
```

Native tests use a fake transport and need no bridge:

```sh
cargo test -p spore-hue
```


## WIT surface

`module.wasm` exports `fungos:hue/lights@0.1.0` alongside the CLI `run`
function. Other Spores can hold a `bridge` resource and read rooms, lamps and
scenes without scraping the CLI:

```wit
let br = hue/lights.bridge.connect("http://hue.local", "default");
let snap = br.snapshot();
br.set-power(hue/lights.target.group("81"), true);
br.set-brightness(hue/lights.target.light("3"), 180);
br.set-colour(hue/lights.target.light("3"), "#ff8800");
br.set-scene("81", "qRFrxxoQ9xCWxv1h");
```

The resource never receives a bridge key. `credential` is a host handle such as
`default` or `geode://home/hue/bridge`; the host leases it and substitutes it
into the v1 URL path. `snapshot` returns the bridge's all-lights group even on
firmware that omits it from `/groups`.

## Soil library

`lib/hue.soil` is the Spore's Hue domain layer in Soil: bridge URL
construction, targets, scenes, brightness that encodes as an integer,
sRGB→CIE-xy conversion, per-lamp gamut clamping, named moods, spatial plans and
timed demos. `lib/cli.soil` is the CLI over it — the same flags, verbs,
rejection messages and summaries as the Rust entry point, so the port can be
measured instead of guessed. Both own Hue semantics only: the host still owns
the origin allowlist and the bridge key, which the guest writes as
`__CREDENTIAL__` and names `default`.

It is the layer other Soil programs call, and the seam `lights-view` and the
CLI port will use:

```lisp
(defun home () (hue/bridge "http://192.168.1.10" "default"))

(hue/on (home) (hue/group "0"))
(hue/bri (home) (hue/light "3") 180)
(hue/scene (home) (hue/group "1") "Relax")
(hue/atmosphere (home) "81" "aurora" nil)
```

The CLI is the same thing with argv in front of it:

```lisp
(hue/cli (list "--bridge" "http://192.168.1.10" "--credential" "default"
               "--group" "1" "scene" "Relax"))
; => (ok "group 1: scene Relax")
```

Load the prelude and `crates/soil-lang/bindings/soil-http.soil` first, then
`lib/hue.soil`, then `lib/cli.soil`. The shipped `module.wasm` is still built
from `src/`, so the Rust CLI and `hue-client` stay until a Spore module can be
built from Soil; see [`docs/hue-soil-port.md`](../../docs/hue-soil-port.md) for
the boundary, the two-implementation rule, and the follow-ups.

```sh
cd spores/hue && ./tests/run.sh             # 41 tests, no bridge, no socket
cd spores/hue && ./tests/check-hue-lib.sh   # the copy still matches loam at the pin
cd spores/hue && ./tests/run.sh --socket    # plus the CLI against a fake bridge
```

`--socket` needs a Soil tree whose host allows the credential-less `pair`
request (the loam tip does); it says so if the selected tree does not.
