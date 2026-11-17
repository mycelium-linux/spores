//! warden: what every repository in the fleet is missing, and what has rotted.
//!
//! The shape is three documents and one function between them:
//!
//! ```text
//! forge            snapshot            report
//! (GraphQL) ──ingest──► (repos) ──evaluate──► (findings) ──render──► text | JSON
//! ```
//!
//! Nothing here opens a socket, reads the clock, or touches a file. A snapshot
//! arrives whole, [`evaluate`] is a pure function of it, and the answer is the
//! same on a laptop, in CI, inside a shell command, and behind a view. That is
//! the whole reason a check is written down once instead of twice.
//!
//! The rules live in [`policy`], the vocabulary in [`check`], the documents in
//! [`snapshot`] and [`report`], and the forge adapter in [`ingest`].

pub mod check;
pub mod date;
pub mod ingest;
pub mod policy;
pub mod report;
pub mod snapshot;

pub use check::{Check, Severity, Status};
pub use policy::{Finding, Policy, evaluate};
pub use report::{
    RepoReport, Report, SkipReason, Skipped, render_json, render_repo, render_text,
    render_text_with_limit,
};
pub use snapshot::{Link, RepoState, Run, SCHEMA, Snapshot};
