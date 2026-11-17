//! `warden`: the command-line face of the jukebox.
//!
//! A `wasi:cli` component, so it runs under `soil-command`, in `mush`, under a
//! shim in zsh, in a GitHub Action, and on a laptop with the same bytes. It
//! imports nothing from the host: the snapshot arrives on stdin and the report
//! leaves on stdout, which is why the command works in a sandbox that links no
//! capability at all.
//!
//! Build for a shell:
//!   cargo build --release --target wasm32-wasip2 --features cli --bin warden

use std::io::{IsTerminal, Read, Write};
use std::process::ExitCode;

use spore_warden::check::{Check, Severity};
use spore_warden::ingest::Collector;
use spore_warden::{Report, Snapshot, evaluate, render_json, render_repo};

const USAGE: &str = "\
warden — what every repository in the fleet is missing, and what has rotted

USAGE
  # collect: GitHub's GraphQL pages in, one snapshot out
  gh api graphql --paginate ... | warden ingest > snapshot.json

  # score: a snapshot in, findings out; the exit code is the answer
  warden report < snapshot.json
  warden report --json < snapshot.json
  warden explain fpl/bob < snapshot.json

  # the vocabulary, and how to read it
  warden checks

OPTIONS
  --json          print the report as JSON instead of text
  --all           list every repository with findings, not just the worst
  --limit N       how many repositories the text listing shows (default 15)
  --strict        treat a check with no evidence as a failure
  --fail-on LEVEL exit non-zero for findings at LEVEL: critical | advisory | none
  --at RFC3339    the collection time to record (default: now)
  -h, --help      this page
  -V, --version   the version

EXIT
  0  nothing at or above --fail-on failed
  1  something did
  2  the input could not be read
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("warden: {error}");
            ExitCode::from(2)
        }
    }
}

fn run(args: &[String]) -> Result<ExitCode, String> {
    let Some(first) = args.first() else {
        return Ok(report(&[]));
    };
    match first.as_str() {
        "-h" | "--help" | "help" => {
            print!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        "-V" | "--version" => {
            println!("warden {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
        "report" => Ok(report(&args[1..])),
        "ingest" => ingest(&args[1..]),
        "checks" => checks(&args[1..]),
        "explain" => explain(&args[1..]),
        // A bare pipeline with flags and no verb is a report; that is the
        // common case and it should not need a word.
        other if other.starts_with('-') => Ok(report(args)),
        other => Err(format!("unknown command {other:?}. Try `warden --help`.")),
    }
}

/// Read everything on stdin.
fn read_stdin() -> Result<Vec<u8>, String> {
    if std::io::stdin().is_terminal() {
        return Err("no snapshot on stdin. Try `warden report < snapshot.json`".into());
    }
    let mut bytes = Vec::new();
    std::io::stdin()
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read stdin: {error}"))?;
    if bytes.is_empty() {
        return Err("the snapshot on stdin was empty".into());
    }
    Ok(bytes)
}

fn report(args: &[String]) -> ExitCode {
    match report_inner(args) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("warden: {error}");
            ExitCode::from(2)
        }
    }
}

fn report_inner(args: &[String]) -> Result<ExitCode, String> {
    let mut json = false;
    let mut all = false;
    let mut strict = false;
    let mut limit = 15usize;
    let mut fail_on = Some(Severity::Critical);

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => json = true,
            "--all" => all = true,
            "--strict" => strict = true,
            "--limit" => {
                index += 1;
                let value = args.get(index).ok_or("--limit N")?;
                limit = value
                    .parse()
                    .map_err(|_| format!("--limit expects a number, got {value:?}"))?;
            }
            "--fail-on" => {
                index += 1;
                let value = args.get(index).ok_or("--fail-on LEVEL")?;
                fail_on = match value.as_str() {
                    "critical" => Some(Severity::Critical),
                    "advisory" => Some(Severity::Advisory),
                    "none" => None,
                    other => {
                        return Err(format!(
                            "--fail-on expects critical, advisory or none, got {other:?}"
                        ));
                    }
                };
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(ExitCode::SUCCESS);
            }
            other => return Err(format!("unknown option {other:?}. Try `warden --help`.")),
        }
        index += 1;
    }

    let bytes = read_stdin()?;
    let snapshot: Snapshot = serde_json::from_slice(&bytes).map_err(|error| {
        // The most likely mistake is a forge page where a snapshot belongs, and
        // naming it saves the reader from reading a serde error.
        let looks_like_forge = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .is_some_and(|value| value.get("data").is_some());
        if looks_like_forge {
            "that is a forge page, not a snapshot. Run `warden ingest` first".to_string()
        } else {
            format!("read snapshot: {error}")
        }
    })?;

    let report = evaluate(&snapshot);

    if json {
        let rendered = render_json(&report).map_err(|error| format!("render: {error}"))?;
        println!("{rendered}");
    } else {
        let limit = if all { None } else { Some(limit) };
        print!("{}", spore_warden::render_text_with_limit(&report, limit));
    }
    let _ = std::io::stdout().flush();

    Ok(exit_for(&report, fail_on, strict))
}

fn exit_for(report: &Report, fail_on: Option<Severity>, strict: bool) -> ExitCode {
    let threshold = match fail_on {
        None => return ExitCode::SUCCESS,
        Some(severity) => severity,
    };
    let failed = report.repos.iter().any(|repo| {
        repo.findings.iter().any(|finding| {
            let fails = finding.status.is_fail() && finding.severity >= threshold;
            let unknown = strict && finding.status.is_unknown() && finding.severity >= threshold;
            fails || unknown
        })
    });
    if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn ingest(args: &[String]) -> Result<ExitCode, String> {
    let mut at = Some(now_rfc3339());
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--at" => {
                index += 1;
                let value = args.get(index).ok_or("--at RFC3339")?;
                at = Some(value.clone());
            }
            "--no-timestamp" => at = None,
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(ExitCode::SUCCESS);
            }
            other => return Err(format!("unknown option {other:?}. Try `warden --help`.")),
        }
        index += 1;
    }

    let bytes = read_stdin()?;
    let mut collector = Collector::new();
    let pages = collector
        .absorb_stream(&bytes)
        .map_err(|error| format!("read forge pages: {error}"))?;
    if pages == 0 {
        return Err(
            "no repositories in that stream. Is the GraphQL query asking for \
             repositoryOwner.repositories?"
                .into(),
        );
    }
    let snapshot = collector.finish(at);
    let rendered = serde_json::to_string_pretty(&snapshot)
        .map_err(|error| format!("render snapshot: {error}"))?;
    println!("{rendered}");
    let _ = std::io::stdout().flush();
    Ok(ExitCode::SUCCESS)
}

fn checks(args: &[String]) -> Result<ExitCode, String> {
    let json = args.iter().any(|arg| arg == "--json");
    if json {
        let entries: Vec<serde_json::Value> = Check::ALL
            .into_iter()
            .map(|check| {
                serde_json::json!({
                    "id": check.id(),
                    "severity": check.severity().id(),
                    "question": check.question(),
                    "remedy": check.remedy(),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&entries).map_err(|error| format!("render: {error}"))?
        );
        return Ok(ExitCode::SUCCESS);
    }
    for check in Check::ALL {
        println!(
            "{:<12} {:<9} {}\n{:<22}→ {}",
            check.id(),
            check.severity().id(),
            check.question(),
            "",
            check.remedy()
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn explain(args: &[String]) -> Result<ExitCode, String> {
    let Some(slug) = args.first().filter(|arg| !arg.starts_with('-')) else {
        return Err("explain needs a repository, for example `warden explain fpl/bob`".into());
    };
    let bytes = read_stdin()?;
    let snapshot: Snapshot =
        serde_json::from_slice(&bytes).map_err(|error| format!("read snapshot: {error}"))?;
    let report = evaluate(&snapshot);

    if let Some(repo) = report
        .repos
        .iter()
        .find(|repo| repo.slug().eq_ignore_ascii_case(slug))
    {
        print!("{}", render_repo(repo));
        return Ok(ExitCode::SUCCESS);
    }
    // A repository that was skipped is still a repository somebody asked about;
    // answering "not found" for one we deliberately ignored would be a lie.
    if let Some(skipped) = report
        .skipped
        .iter()
        .find(|repo| repo.slug().eq_ignore_ascii_case(slug))
    {
        println!("{}\n  skipped: {}", skipped.slug(), skipped.reason.id());
        return Ok(ExitCode::SUCCESS);
    }
    Err(format!("{slug} is not in this snapshot"))
}

/// Now, as WASI tells it. A wasip2 command has `wasi:clocks`, so this is the
/// guest's own clock and not a host import fabricated for the purpose.
fn now_rfc3339() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default();
    format_rfc3339(seconds)
}

fn format_rfc3339(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

/// The inverse of `days_from_civil`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::{civil_from_days, format_rfc3339};
    use spore_warden::check::{Check, Status};
    use spore_warden::policy::{Finding, Policy, evaluate};
    use spore_warden::report::{RepoReport, Report};
    use spore_warden::{RepoState, Snapshot, render_text_with_limit};

    #[test]
    fn the_clock_round_trips_through_the_civil_calendar() {
        assert_eq!(format_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_rfc3339(1_794_912_854), "2026-11-17T10:54:14Z");
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    fn report_with(names: &[&str]) -> Report {
        let repos: Vec<RepoReport> = names
            .iter()
            .map(|name| RepoReport {
                org: "fpl".into(),
                name: (*name).into(),
                findings: vec![Finding {
                    check: Check::License,
                    severity: Check::License.severity(),
                    status: Status::Fail {
                        detail: "no licence".into(),
                    },
                }],
            })
            .collect();
        Report::new(
            1,
            Some("2026-11-17T00:00:00Z".into()),
            Policy::default(),
            vec!["fpl".into()],
            repos,
            vec![],
        )
    }

    #[test]
    fn the_listing_stops_at_the_limit_and_says_how_many_are_left() {
        let report = report_with(&["a", "b", "c", "d"]);
        let text = render_text_with_limit(&report, Some(2));
        assert!(text.contains("fpl/a"), "{text}");
        assert!(text.contains("fpl/b"), "{text}");
        assert!(!text.contains("fpl/c"), "{text}");
        assert!(text.contains("2 more with findings"), "{text}");
    }

    #[test]
    fn no_limit_lists_everything() {
        let report = report_with(&["a", "b", "c", "d"]);
        let text = render_text_with_limit(&report, None);
        assert!(text.contains("fpl/d"), "{text}");
        assert!(!text.contains("more with findings"), "{text}");
    }

    #[test]
    fn a_clean_snapshot_still_produces_a_report() {
        let snapshot = Snapshot {
            schema: 1,
            collected_at: Some("2026-11-17T00:00:00Z".into()),
            policy: Policy::default(),
            repos: vec![RepoState {
                org: "fpl".into(),
                name: "bob".into(),
                license: Some("MIT".into()),
                workflows: vec!["ci.yml".into()],
                ..RepoState::default()
            }],
        };
        let report = evaluate(&snapshot);
        assert_eq!(report.repos.len(), 1);
    }
}
