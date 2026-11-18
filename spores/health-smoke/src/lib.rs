//! Health smoke: answer every request with 200 and count it.
//!
//! The Spore exists to make the runner's two claims testable from the outside —
//! `GET /metrics` renders the registry the guest wrote, and a `wasi:http`
//! component that imports `soil:metrics` instantiates at all — so it does
//! nothing else. No request parsing, no body, no state beyond what the host
//! registry accumulates.

wit_bindgen::generate!({
    path: "wit",
    world: "health-smoke",
    generate_all,
    with: { "soil:metrics/metrics@0.1.0": metrics_sdk::api },
});

use metrics_sdk::Counter;
use wasi::http::types::{
    Fields, IncomingRequest, OutgoingBody, OutgoingResponse, ResponseOutparam,
};

struct Component;

export!(Component);

impl exports::wasi::http::incoming_handler::Guest for Component {
    fn handle(_request: IncomingRequest, response_out: ResponseOutparam) {
        // A metric is a fact about work that already happened, so it must never
        // fail the work: a refused write (no grant, a bad name) leaves the 200
        // intact and the scrape empty, which is what a probe can distinguish.
        let mut answered = Counter::new(
            "health_smoke_answered_total",
            Some("Requests health-smoke answered with 200"),
            &[],
        );
        let _ = answered.add(1);

        answer(response_out);
    }
}

fn answer(response_out: ResponseOutparam) {
    let response = OutgoingResponse::new(Fields::new());
    if response.set_status_code(200).is_err() {
        return;
    }
    let Ok(body) = response.body() else {
        return;
    };
    // Handing the response to the host before writing the body is the order the
    // ABI expects: the outparam owns the response from here, and the body
    // stream is what the guest still holds.
    ResponseOutparam::set(response_out, Ok(response));
    let Ok(stream) = body.write() else {
        return;
    };
    let _ = stream.blocking_write_and_flush(b"ok\n");
    drop(stream);
    let _ = OutgoingBody::finish(body, None);
}
