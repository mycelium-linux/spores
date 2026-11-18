# metrics-sdk

Typed `Counter` and `Gauge` over `soil:metrics`, for Spore guests.

```rust
wit_bindgen::generate!({
    path: "wit",
    world: "my-spore",
    with: { "soil:metrics/metrics@0.1.0": metrics_sdk::api },
});

use metrics_sdk::{Counter, Gauge};

let mut served = Counter::new("my_spore_served_total", Some("Requests served"), &[]);
let _ = served.add(1);

let mut depth = Gauge::new("my_spore_queue_depth", None, &[("queue", "draft")]);
let _ = depth.set(3);
```

Why it is this small: the host owns the registry, the endpoint and the scrape
budget, so a Spore names a series and a value and nothing else. `new` is
infallible and `add`/`set` return the host's refusal, because the budget is the
host's rule — a second copy of it compiled into every Spore is what would drift.

A metric must never fail the work it describes. A caller that can act on
`Error::Denied` or `Error::Invalid` should; one that cannot may drop it on the
floor, and the request still succeeds.

The interface is generated once, here, and handed to the guest's world with
`with:`. The Spore, this crate and the host then share one binding.

## Vendored contract

`wit/deps/metrics.wit` is a byte copy of `soil`'s `wit/soil-metrics.wit`, recorded
in the repository's `wit-provenance.txt` with its blob. Bumping it means copying
soil's file over this one and updating that row in the same commit.
