//! Typed counters and gauges over `soil:metrics`, for Spore guests.
//!
//! The host owns the registry and answers `GET /metrics` in Prometheus text on
//! the Spore's own vhost. A Spore only names a series and a value, which is why
//! this crate is small: there is no client, no endpoint and no scrape budget to
//! configure, because none of those are the Spore's to decide.
//!
//! The interface is generated here and handed to the guest's world with
//! `with:`, so the Spore, this crate and the host speak one binding:
//!
//! ```ignore
//! wit_bindgen::generate!({
//!     path: "wit",
//!     world: "my-spore",
//!     with: { "soil:metrics/metrics@0.1.0": metrics_sdk::api },
//! });
//! ```
//!
//! Labels are a slice of pairs rather than a map: the host sorts them, so two
//! calls that differ only in order are one series, and a `BTreeMap` at every
//! call site would be ceremony over two labels.

wit_bindgen::generate!({
    path: "wit",
    world: "metrics-sdk",
    generate_all,
});

/// The generated `soil:metrics/metrics` interface.
///
/// A guest maps this module into its own world; `api::emit` and `api::snapshot`
/// are then the guest's own imports.
pub use soil::metrics::metrics as api;

/// Why the host refused a write: a name or label outside the budget, the
/// series budget exhausted, or no `soil:metrics` grant in the manifest.
pub type Error = api::Error;

fn sample(
    name: impl Into<String>,
    kind: api::Kind,
    help: Option<&str>,
    labels: &[(&str, &str)],
) -> api::Sample {
    api::Sample {
        name: name.into(),
        kind,
        value: 0.0,
        help: help.map(str::to_owned),
        labels: labels
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect(),
    }
}

/// A series that only goes up.
///
/// Each `add` is one write to the host registry; the host adds it to whatever
/// the series already held. A Spore that keeps a `Counter` in a `thread_local!`
/// does not have to remember the total itself — the host is the accumulator.
pub struct Counter {
    sample: api::Sample,
}

impl Counter {
    /// Name a series. Infallible on purpose: the budget is the host's rule, so
    /// the refusal arrives from the write that breaks it rather than from a
    /// second copy of the rule compiled into every Spore.
    #[must_use]
    pub fn new(name: impl Into<String>, help: Option<&str>, labels: &[(&str, &str)]) -> Self {
        Self {
            sample: sample(name, api::Kind::Counter, help, labels),
        }
    }

    /// Add `by` to this series.
    ///
    /// Returns the host's refusal instead of panicking or silently dropping it:
    /// a caller that can do something about a bad name should see it, and one
    /// that cannot can ignore it without the request failing.
    ///
    /// # Errors
    /// [`Error::Invalid`], [`Error::OverBudget`], or [`Error::Denied`].
    pub fn add(&mut self, by: impl Into<f64>) -> Result<(), Error> {
        self.sample.value = by.into();
        api::emit(std::slice::from_ref(&self.sample))
    }
}

/// A series that may go up or down. Each `set` replaces the value the host
/// holds for the series, which is what a queue depth or a process count means.
pub struct Gauge {
    sample: api::Sample,
}

impl Gauge {
    #[must_use]
    pub fn new(name: impl Into<String>, help: Option<&str>, labels: &[(&str, &str)]) -> Self {
        Self {
            sample: sample(name, api::Kind::Gauge, help, labels),
        }
    }

    /// Replace this series' value.
    ///
    /// # Errors
    /// [`Error::Invalid`], [`Error::OverBudget`], or [`Error::Denied`].
    pub fn set(&mut self, value: impl Into<f64>) -> Result<(), Error> {
        self.sample.value = value.into();
        api::emit(std::slice::from_ref(&self.sample))
    }
}
