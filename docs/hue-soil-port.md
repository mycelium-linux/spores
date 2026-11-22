# Hue: the Soil domain layer inside the Spore

Status: library and tests folded in; the CLI entry point is still Rust.

The Spore port plan makes Hue the first real port. This change takes the part
that was written outside the Spore — loam's `examples/hue/hue.soil` — and puts
it where its consumers can find it: `spores/hue/lib/hue.soil`, with a test
corpus beside it. The Rust CLI keeps shipping; it now has a Soil sibling that
owns the same semantics for every other caller.

## What landed

```text
spores/hue/lib/hue.soil                  bridge URL, targets, scenes, colour, moods
spores/hue/tests/soil/hue.soil           colour, planning and restore tests (loam corpus)
spores/hue/tests/soil/hue_request.soil   request shaping and bridge error decoding
spores/hue/tests/run.sh                  runner: prelude + soil:http binding + both test files
```

`lib/hue.soil` is byte-identical to loam's example at `86a1296`
(`pi/hue-int-bri`), which is the tip of the Hue work: colour theory
(`hue/harmony`, `hue/hsl->rgb`), gamut-aware palettes (`hue/clamp-gamut`), named
moods, spatial plans over a room layout, and timed demos that restore the room.
Keeping it identical means a future sync is a copy, not a merge, and the
comment in its header that points at `examples/hue/README.md` still refers to
the loam checkout that produced it.

Nineteen tests run against a disposable workspace with no bridge and no socket.

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
./tests/run.sh                 # probes ../loam, ../soil, ../soil-dsp, ../../singularity/soil
SOIL=/path/to/soil ./tests/run.sh
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

## What is deliberately still Rust

- **The CLI entry point.** `src/lib.rs` parses `argv`, formats the one-line
  summary, and maps failures to exit codes; `hue-client` supplies the HTTP
  shaping and the sRGB conversion. The Soil library supersedes the second half
  and not yet the first, so porting the entry point is the next step, not this
  one. When it lands, `hue-client` is retired in the same change that removes
  its last consumer (the plan's retirement rule), together with `lights-core`
  once `lights-view` calls the Soil library.
- **The `wasm32-wasip2` component.** `module.wasm` is built from `src/guest.rs`
  over `soil:http`; a Soil-sourced Spore module is a packaging question the
  tree has not answered yet.

## Follow-ups, named

1. Point loam's `examples/hue` at this file — a copy is a poor long-term
   relationship, and the example should not be able to drift.
2. Port the CLI entry point to Soil and delete `hue-client` with it.
3. Make `lights-view` call `hue/` functions instead of reimplementing state.
4. Add a fake-bridge socket test in the Spore tree once the packaging path can
   run a Soil program against a local listener.
