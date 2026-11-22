# Hue: the Soil domain layer inside the Spore

Status: library, CLI and tests are in Soil; the shipped module is still Rust.

The Spore port plan makes Hue the first real port. This change takes the part
that was written outside the Spore — loam's `examples/hue/hue.soil` — and puts
it where its consumers can find it: `spores/hue/lib/hue.soil`, with a test
corpus beside it. The Rust CLI keeps shipping; it now has a Soil sibling that
owns the same semantics for every other caller.

## What landed

```text
spores/hue/lib/hue.soil                  bridge URL, targets, scenes, colour, moods
spores/hue/lib/cli.soil                  argv parsing, dispatch and the one-line summaries
spores/hue/tests/soil/hue.soil           colour, planning and restore tests (loam corpus)
spores/hue/tests/soil/hue_request.soil   request shaping and bridge error decoding
spores/hue/tests/soil/hue_cli.soil       parser, rejection messages and summary shaping
spores/hue/tests/run.sh                  runner: prelude + soil:http binding + library + tests
```

`lib/hue.soil` is byte-identical to loam's example at `86a1296`
(`pi/hue-int-bri`), which is the tip of the Hue work: colour theory
(`hue/harmony`, `hue/hsl->rgb`), gamut-aware palettes (`hue/clamp-gamut`), named
moods, spatial plans over a room layout, and timed demos that restore the room.
Keeping it identical means a future sync is a copy, not a merge, and the
comment in its header that points at `examples/hue/README.md` still refers to
the loam checkout that produced it.

Forty-one tests run against a disposable workspace with no bridge and no socket;
`--socket` adds twelve more checks against a local HTTP stub.

## Boundary

The split is the one the Spore already documents, now enforced in Soil:

- the Spore owns Hue semantics: which resource a verb addresses, how `#RRGGBB`
  becomes CIE xy, which lamp can show a colour, how a mood becomes per-light
  writes, and how the bridge's in-band `[{"error": ...}]` replies become
  `(err "...")`;
- the host owns the connection: the origin allowlist, the byte budget, the
  timeout, and the bridge key. The guest writes the literal `__CREDENTIAL__`
  into the URL path and names the handle (`default`); the host substitutes the
  secret after the allowlist check.

`hue/pair` is the one call with no credential, because pairing *is* the
exchange that mints one.

## Running the tests

```sh
cd spores/hue
./tests/run.sh                 # pure: probes ../loam, ../soil, ../soil-dsp, ../../singularity/soil
./tests/run.sh --socket        # pure, then the CLI against tests/fake_bridge.py
SOIL=/path/to/soil ./tests/run.sh [--socket]
```

The runner needs a Soil tree with `crates/soil-lang/stdlib/prelude.soil` and
the generated `crates/soil-lang/bindings/soil-http.soil`, plus a built
`soil-lang` (override with `SOIL_LANG`). A failing test prints the Soil error
under the test name and the runner exits non-zero.

The tests deliberately avoid `soil:http/fetch`. `hue/request` is a pure
function, so the URL, method, headers, JSON body and credential handle a write
would send are asserted directly; `hue/check` is fed synthetic responses, so an
in-band bridge error and an HTTP error are both covered. That keeps the suite
runnable anywhere, and leaves the socket-level tests where the host lives:
loam's `crates/soil-lang/tests/http.rs` covers the allowlist and the Geode
lease against a real local socket.

## Using it from a Soil program

```lisp
(defun home () (hue/bridge "http://192.168.1.10" "default"))

(hue/on (home) (hue/group "0"))
(hue/bri (home) (hue/light "3") 180)
(hue/color-hex (home) (hue/light "3") "#ff8800")
(hue/scene (home) (hue/group "1") "Relax")
(hue/atmosphere (home) "81" "aurora" (living-room-layout))
```

Load the prelude, `crates/soil-lang/bindings/soil-http.soil`, then
`spores/hue/lib/hue.soil`. Loam's `examples/hue/README.md` carries the REPL
invocation, the Geode lease check, and the mood/layout/demo catalogue; that
prose stays there until the example itself becomes a thin wrapper over this
file.

## The CLI, in Soil

`lib/cli.soil` is the entry point written against `lib/hue.soil` instead of
`hue-client`. `(hue/cli (argv))` parses the same flags, refuses the same
combinations with the same strings, and prints the same summaries as
`src/lib.rs`:

```lisp
(hue/cli (list "--bridge" "http://192.168.1.10" "--credential" "default"
               "--light" "3" "color" "#ff8800"))
; => (ok "light 3: #ff8800 -> xy (0.601, 0.3837)")
```

Parsing is pure — `hue/parse-argv` returns `(ok <command>)` or
`(err "message")` — so every rejection path is a unit test, including the ones
that are easy to get wrong: an empty `--key` counting as absent, `--key` plus
`--credential` being a conflict rather than a precedence question, `bri 255`
parsing and then failing the range check, `pair` refusing a credential, and
`--light` with `--group`.

Three behaviours are worth naming because they are Soil-shaped rather than
Rust-shaped:

- `--json` prints compact JSON (`json-encode`), not serde's pretty printer.
  It round-trips through `jq`; the pretty form was cosmetic.
- Ids sort numerically when they parse as numbers, because bridge ids are
  numbers; anything else keeps bridge order. Rust sorted keys as strings, which
  ordered `"10"` before `"2"`.
- Decoded JSON object keys arrive as keywords (`:1`), so `hue/key-text` drops
  the sigil before an id is used in a URL or a body.

Everything else is parity, including the rejection strings, which the tests
pin word for word.

## The socket suite

`./tests/run.sh --socket` starts `tests/fake_bridge.py` on a free port and runs
`tests/soil/hue_socket.soil` through the real host:

```sh
soil-lang --allow "$HUE_BRIDGE" --http-credential test-key \
  <prelude> <soil-http binding> lib/hue.soil lib/cli.soil \
  tests/soil/hue_socket.soil --call test/socket-status
```

The stub answers the routes the library uses, keeps mutable state so `toggle`
has something to invert, and **only accepts the key the host leased**. That is
the assertion worth having: the Soil source writes `__CREDENTIAL__`, the host
substitutes `test-key`, and a green run means no secret ever lived in the
guest. The stub also logs every request, so `socket.sh` checks what crossed the
wire — `PUT /api/test-key/groups/1/action {"scene": "7"}` — and fails if the
placeholder appears in the log.

The ten socket tests cover `status`, `lights` order, `on`, `bri`/`color`,
`toggle`, a case-insensitive `scene`, a missing scene, an in-band bridge error
reaching the CLI as `(err "bridge: ...")`, compact `--json`, and `pair`.

`pair` is the reason the socket suite names a host requirement. It is the one
request with no credential handle, and an older `NativeHost` (the `soil-dsp`
checkout, say) always injects a key and answers `(err denied)`. `socket.sh`
probes for that before running anything and exits 2 with the reason, and
`run.sh --socket` turns that into an actionable message instead of a Hue
failure: set `SOIL` to a tree whose native `soil:http` allows a credential-less
request (the loam tip does). The pure suite needs no such host.

## What is deliberately still Rust

- **The shipped module.** `module.wasm` is still built from `src/lib.rs` and
  `src/guest.rs` over `soil:http`; `hue-client` is still its HTTP and colour
  layer. The entry logic now exists twice on purpose: `lib/cli.soil` is the
  reference implementation the port is measured against, and `src/lib.rs` is
  what the package can actually launch today. `hue-client` retires in the same
  change that builds the module from Soil (the plan's retirement rule), not
  before; until then the CLI tests are the thing that keeps the two honest.
- **Module packaging.** A Soil-sourced Spore module is a packaging question the
  tree has not answered yet: `[module] path = "module.wasm"` assumes a compiled
  component, and Soil's own runner is a host binary. That is the blocker for
  deleting `src/`, and it is a tree-level decision, not a Hue one.

## Follow-ups, named

1. Point loam's `examples/hue` at this file — a copy is a poor long-term
   relationship, and the example should not be able to drift.
2. Make `lights-view` call `hue/` functions instead of reimplementing state.
3. Decide module packaging for a Soil-sourced Spore, then delete `src/` and
   `hue-client` together.
