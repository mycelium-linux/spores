//! The document contract, end to end.
//!
//! The unit tests prove the checks; this one proves the *pipeline*. It starts
//! from the bytes `gh` actually emits, runs the same two steps the shell runs,
//! and then reads the report back as JSON — because a view consumes that file
//! and never calls this crate.

use spore_warden::check::Check;
use spore_warden::ingest::Collector;
use spore_warden::{Report, Snapshot, evaluate, render_text};

const FORGE: &str = include_str!("fixtures/forge-pages.json");

fn snapshot() -> Snapshot {
    let mut collector = Collector::new();
    let pages = collector
        .absorb_stream(FORGE.as_bytes())
        .expect("the fixture is what gh emits");
    assert_eq!(pages, 3, "every repository on every page is read");
    collector.finish(Some("2026-11-17T12:00:00Z".into()))
}

#[test]
fn a_forge_stream_becomes_a_snapshot_a_shell_could_have_saved() {
    let snapshot = snapshot();
    assert_eq!(snapshot.orgs(), vec!["fpl".to_string()]);
    assert_eq!(snapshot.repos.len(), 3);
    // Sorted, so two collections of the same fleet are the same file.
    let names: Vec<&str> = snapshot
        .repos
        .iter()
        .map(|repo| repo.name.as_str())
        .collect();
    assert_eq!(names, vec!["bare", "dead", "healthy"]);

    let json = serde_json::to_string(&snapshot).expect("a snapshot is JSON");
    let back: Snapshot = serde_json::from_str(&json).expect("and reads back");
    assert_eq!(snapshot, back);
}

#[test]
fn the_three_repositories_land_in_the_three_states() {
    let report = evaluate(&snapshot());
    assert_eq!(report.summary.checked, 2, "the archived one is not scored");
    assert_eq!(report.summary.skipped, 1);
    assert_eq!(report.summary.critical, 1, "bare fails licence and ci");
    assert_eq!(report.summary.advisory, 0, "healthy fails nothing");
    // `clean` is stricter than `worst_fail().is_none()` on purpose: healthy has
    // no failing check and two unmeasured ones, so it is not clean, it is
    // unmeasured. That is the rule, stated where a reader will trip over it.
    assert_eq!(report.summary.clean, 0);
    assert_eq!(report.summary.unknown, 2);

    let healthy = report
        .repos
        .iter()
        .find(|repo| repo.name == "healthy")
        .expect("healthy is scored");
    assert!(healthy.worst_fail().is_none(), "{healthy:?}");

    let bare = report
        .repos
        .iter()
        .find(|repo| repo.name == "bare")
        .expect("bare is scored");
    let failed: Vec<&str> = bare.fails().map(|finding| finding.check.id()).collect();
    assert!(failed.contains(&Check::License.id()), "{failed:?}");
    assert!(failed.contains(&Check::Ci.id()), "{failed:?}");
    assert!(
        !failed.contains(&Check::Branches.id()),
        "three branches is not a problem"
    );
}

#[test]
fn the_report_json_carries_what_a_view_needs_without_recomputing() {
    let report = evaluate(&snapshot());
    let value: serde_json::Value =
        serde_json::from_str(&report::render_json(&report).unwrap()).expect("the report is JSON");

    for key in [
        "schema",
        "collected_at",
        "policy",
        "orgs",
        "summary",
        "repos",
        "skipped",
    ] {
        assert!(value.get(key).is_some(), "a view reads {key}");
    }
    let summary = &value["summary"];
    assert_eq!(summary["checked"], 2);
    assert_eq!(
        summary["by_check"].as_array().map(Vec::len),
        Some(Check::ALL.len())
    );

    // Every finding is tagged with its status, so a view renders four buckets
    // rather than inferring a pass from an absent failure.
    let finding = &value["repos"][0]["findings"][0];
    assert!(finding.get("check").is_some());
    assert!(finding.get("severity").is_some());
    assert!(finding.get("status").is_some());
}

#[test]
fn the_report_serialises_and_reads_back_so_a_view_can_cache_it() {
    let report = evaluate(&snapshot());
    let json = report::render_json(&report).unwrap();
    let back: Report = serde_json::from_str(&json).expect("the report document round trips");
    assert_eq!(report, back);
}

#[test]
fn the_text_a_person_reads_names_the_owners_the_scope_and_the_remedy() {
    let report = evaluate(&snapshot());
    let text = render_text(&report);
    assert!(text.contains("warden: fpl"), "{text}");
    assert!(text.contains("2 repositories"), "{text}");
    assert!(text.contains("fpl/bare"), "{text}");
    assert!(text.contains("no licence file"), "{text}");
    assert!(
        text.contains("unknown is not a pass"),
        "the reader has to be told what the blank column means"
    );
}

use spore_warden::report;
