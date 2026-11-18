# board

The Marbles work graph as one surface, rendered by **GPUI inside a
`wasm32-wasip2` component**. The Spore owns the view; the host owns the WebGPU
device, the compositor surface and the records book. No window, swapchain or
native handle crosses that boundary.

**Search**, a **kanban** by status, and a **burndown** against an even pace —
all from `soil:records`, with no network and no filesystem of its own.

```sh
cargo build --release --target wasm32-wasip2

soil spore run . --collection marbles --book marbles
```

## How it runs

```
host: wasi:webgpu instance + soil:records book + soil:present surface
  └─ guest: records::scan("marbles") → issues
       └─ WgpuContext::from_offscreen
            └─ HeadlessAppContext + Board view (GPUI + gpui-kit)
                 └─ RGBA frame → host texture → present
```

Keys: printable text filters, `tab` switches board/burndown, `escape` clears
the search and then quits, `⌘R` re-reads the store.

## The two halves

`src/model.rs` is the whole of the arithmetic — decoding records into issues,
grouping the columns, folding the burndown, matching a search. It has no GPU or
component dependency, so `cargo test` exercises it on the host in
milliseconds:

```sh
cargo test        # 13 tests over the projections
```

`src/spore.rs` is the GPUI lane and only exists for `wasm32`. It is the part
that cannot be checked without a device, so it stays as thin as an honest
renderer can: it lays out, draws one frame, and forwards keys and pointers.

## Why the collection is on the command line

Marbles keeps every record type in one collection literally named `marbles`,
with `issue/[...]` entities inside it. The manifest's `[records]` table names
declared collections as schema ids (`area/name@version`), which `marbles` is
not — so the operator names it with `--collection`, the same flag that is the
grant to open it.

## Read-only by grant

The board holds `soil:records` and nothing narrower, and never writes: the
tracker stays the single writer of its own graph. A board that could also move
a card would be a second authority beside the store it is drawing.
