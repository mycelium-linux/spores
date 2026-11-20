//! The server-rendered pages.
//!
//! One function per route, each returning a complete HTML document. There is no
//! client-side state, no JSON fetch and no framework: the guest reads the book,
//! computes the projections in [`crate::model`], and writes the page. What the
//! browser receives is what the book said at that moment.
//!
//! Every interpolation that can contain operator text goes through
//! [`escape`]. Issue titles and descriptions are written by people and agents,
//! and a view that pastes them into markup unescaped is an injection as soon as
//! one of them contains a `<`.

use std::fmt::Write as _;

use crate::model::{
    self, date_label, plan_name, Book, Burndown, Column, Issue, PlanStat, ProjectStat, Summary,
};

/// The pages the header links to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Dashboard,
    Board,
    Burndown,
    Projects,
    Plans,
}

impl Page {
    fn path(self) -> &'static str {
        match self {
            Page::Dashboard => "/",
            Page::Board => "/board",
            Page::Burndown => "/burndown",
            Page::Projects => "/projects",
            Page::Plans => "/plans",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Page::Dashboard => "Overview",
            Page::Board => "Board",
            Page::Burndown => "Burndown",
            Page::Projects => "Projects",
            Page::Plans => "Plans",
        }
    }
}

fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Encode one query value. Alpha-numeric and a handful of safe characters pass
/// through; everything else is percent-encoded byte by byte.
fn url_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            _ => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

fn thousands(value: i64) -> String {
    let negative = value < 0;
    let digits = value.abs().to_string();
    let mut out = String::new();
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if negative {
        format!("-{out}")
    } else {
        out
    }
}

/// A query string carrying the current filter and a page.
fn link(path: &str, project: Option<&str>, extra: &[(&str, &str)]) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(project) = project.filter(|project| !project.is_empty()) {
        parts.push(format!("project={}", url_encode(project)));
    }
    for (key, value) in extra {
        if !value.is_empty() {
            parts.push(format!("{}={}", url_encode(key), url_encode(value)));
        }
    }
    if parts.is_empty() {
        path.to_string()
    } else {
        format!("{path}?{}", parts.join("&"))
    }
}

/// The shared chrome: header, filter form, project picker, and the page body.
pub struct Chrome<'a> {
    pub book: &'a Book,
    pub project: Option<&'a str>,
    pub query: &'a str,
    pub page: Page,
}

impl Chrome<'_> {
    fn options(&self) -> String {
        let mut out = String::from("<option value=\"\">All projects</option>");
        for slug in model::project_slugs(self.book) {
            let selected = if self.project == Some(slug.as_str()) {
                " selected"
            } else {
                ""
            };
            let _ = write!(
                out,
                "<option value=\"{}\"{selected}>{}</option>",
                escape(&slug),
                escape(&slug)
            );
        }
        out
    }

    fn nav(&self) -> String {
        let mut out = String::new();
        for page in [
            Page::Dashboard,
            Page::Board,
            Page::Burndown,
            Page::Projects,
            Page::Plans,
        ] {
            let class = if page == self.page {
                " class=\"on\""
            } else {
                ""
            };
            let _ = write!(
                out,
                "<a href=\"{}\"{class}>{}</a>",
                link(page.path(), self.project, &[]),
                page.title()
            );
        }
        out
    }

    /// Render `body` inside the document.
    pub fn wrap(&self, heading: &str, sub: &str, body: &str) -> String {
        let title = if heading.is_empty() || heading == "Marbles" {
            "Marbles".to_string()
        } else {
            format!("{} — Marbles", escape(heading))
        };
        format!(
            r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>{CSS}</style>
</head>
<body>
<header class="top">
  <a class="brand" href="{home}"><span class="mark">M</span> Marbles</a>
  <nav>{nav}</nav>
  <form class="find" method="get" action="{board}">
    <input type="search" name="q" value="{query}" placeholder="search id, title, label, holder" aria-label="Search issues">
    <select name="project" aria-label="Project">{options}</select>
    <button type="submit">Search</button>
  </form>
</header>
<main>
  <div class="head">
    <h1>{heading}</h1>
    <p class="sub">{sub}</p>
  </div>
  {body}
</main>
<footer>
  <span>read-only view of <code>soil:records</code></span>
  <span>now = newest record in the book · {now}</span>
</footer>
</body>
</html>
"#,
            CSS = CSS,
            home = link("/", self.project, &[]),
            nav = self.nav(),
            board = link("/board", None, &[]),
            query = escape(self.query),
            options = self.options(),
            heading = escape(heading),
            sub = sub,
            body = body,
            now = escape(&date_label(self.book.now)),
        )
    }
}

const CSS: &str = r#"
:root{--bg:#0b0f14;--panel:#121922;--panel2:#0f151d;--edge:#222e3a;--ink:#e6edf5;--muted:#8b98a6;--dim:#5d6b7a;--accent:#6ea8fe;--good:#7ad0a3;--warn:#ffd166;--bad:#ff7b72;--violet:#c4a7ff}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--ink);font:14px/1.5 ui-sans-serif,-apple-system,BlinkMacSystemFont,"Segoe UI",system-ui,sans-serif}
a{color:var(--accent);text-decoration:none}
a:hover{text-decoration:underline}
code{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:12px;color:var(--muted)}
header.top{position:sticky;top:0;z-index:5;display:flex;align-items:center;gap:18px;flex-wrap:wrap;padding:10px 22px;background:rgba(11,15,20,.92);backdrop-filter:blur(8px);border-bottom:1px solid var(--edge)}
.brand{display:flex;align-items:center;gap:9px;color:var(--ink);font-weight:650;letter-spacing:.2px}
.brand:hover{text-decoration:none}
.mark{display:grid;place-items:center;width:26px;height:26px;border-radius:8px;background:linear-gradient(140deg,#6ea8fe,#c4a7ff);color:#0b0f14;font-weight:800}
header nav{display:flex;gap:4px;flex-wrap:wrap}
header nav a{padding:5px 11px;border-radius:8px;color:var(--muted);font-weight:550}
header nav a:hover{background:var(--panel);text-decoration:none;color:var(--ink)}
header nav a.on{background:var(--panel);color:var(--ink);box-shadow:inset 0 0 0 1px var(--edge)}
.find{margin-left:auto;display:flex;gap:6px;align-items:center;flex-wrap:wrap}
.find input,.find select,.find button{background:var(--panel2);color:var(--ink);border:1px solid var(--edge);border-radius:8px;padding:6px 9px;font:inherit}
.find input{min-width:230px}
.find button{cursor:pointer;background:var(--panel);color:var(--muted)}
.find button:hover{color:var(--ink)}
main{max-width:1280px;margin:0 auto;padding:22px}
.head h1{margin:6px 0 2px;font-size:24px;letter-spacing:-.2px}
.sub{margin:0 0 18px;color:var(--muted)}
.tiles{display:grid;grid-template-columns:repeat(auto-fit,minmax(140px,1fr));gap:10px;margin-bottom:18px}
.tile{background:var(--panel);border:1px solid var(--edge);border-radius:12px;padding:12px 14px}
.tile .k{color:var(--muted);font-size:11px;text-transform:uppercase;letter-spacing:.8px}
.tile .v{font-size:24px;font-weight:650;margin-top:2px}
.tile .v.good{color:var(--good)} .tile .v.warn{color:var(--warn)} .tile .v.bad{color:var(--bad)} .tile .v.accent{color:var(--accent)}
.panel{background:var(--panel);border:1px solid var(--edge);border-radius:14px;padding:16px;margin-bottom:18px}
.panel h2{margin:0 0 4px;font-size:15px;letter-spacing:.2px}
.panel .hint{margin:0 0 12px;color:var(--muted);font-size:12px}
.board{display:grid;grid-template-columns:repeat(6,minmax(170px,1fr));gap:10px;overflow-x:auto;padding-bottom:6px}
.col{background:var(--panel2);border:1px solid var(--edge);border-radius:12px;padding:10px;min-height:120px}
.col h3{margin:0 0 8px;font-size:12px;text-transform:uppercase;letter-spacing:.7px;color:var(--muted);display:flex;justify-content:space-between}
.col h3 .n{color:var(--dim)}
.card{background:var(--panel);border:1px solid var(--edge);border-radius:9px;padding:9px 10px;margin-bottom:8px}
.card:hover{border-color:#31445a}
.card .id{font-family:ui-monospace,Menlo,monospace;font-size:11px;color:var(--dim)}
.card .title{color:var(--ink);display:block;margin:1px 0 5px;line-height:1.35}
.chips{display:flex;flex-wrap:wrap;gap:4px}
.chip{font-size:10px;padding:1px 6px;border-radius:999px;background:#1a2431;color:var(--muted);border:1px solid var(--edge)}
.chip.p1{color:#ffb4a9;border-color:#4a2a2a}
.chip.label{color:var(--violet);border-color:#3a3350}
.chip.who{color:var(--good);border-color:#244335}
table{width:100%;border-collapse:collapse;font-size:13px}
th,td{text-align:left;padding:7px 8px;border-bottom:1px solid var(--edge)}
th{color:var(--muted);font-size:11px;text-transform:uppercase;letter-spacing:.6px;font-weight:600}
td.num,th.num{text-align:right;font-variant-numeric:tabular-nums}
tr:hover td{background:var(--panel2)}
.bar{height:6px;border-radius:999px;background:#1a2431;overflow:hidden;min-width:80px}
.bar i{display:block;height:100%;background:linear-gradient(90deg,#7ad0a3,#6ea8fe)}
.chart{width:100%;height:auto;display:block}
.legend{display:flex;gap:16px;flex-wrap:wrap;color:var(--muted);font-size:12px;margin-top:6px}
.legend i{display:inline-block;width:16px;height:3px;border-radius:2px;margin-right:6px;vertical-align:middle}
.grid2{display:grid;grid-template-columns:repeat(auto-fit,minmax(320px,1fr));gap:16px}
.kv{display:grid;grid-template-columns:130px 1fr;gap:6px 12px;font-size:13px}
.kv dt{color:var(--muted)}
.kv dd{margin:0;min-width:0;overflow-wrap:anywhere}
.status{font-size:11px;padding:2px 8px;border-radius:999px;border:1px solid var(--edge);color:var(--muted);text-transform:uppercase;letter-spacing:.5px}
.status.open{color:var(--accent);border-color:#26455f}
.status.in_progress{color:var(--good);border-color:#244335}
.status.review{color:var(--violet);border-color:#3a3350}
.status.blocked{color:var(--bad);border-color:#4a2a2a}
.status.closed{color:var(--dim)}
.status.deferred{color:var(--warn);border-color:#4a4226}
.ready{color:var(--good)} .notready{color:var(--warn)}
ul.plain{list-style:none;margin:0;padding:0}
ul.plain li{padding:4px 0;border-bottom:1px solid var(--edge)}
footer{max-width:1280px;margin:0 auto;padding:8px 22px 28px;display:flex;justify-content:space-between;gap:12px;color:var(--dim);font-size:11px;flex-wrap:wrap}
.empty{color:var(--dim);font-style:italic}
.error{border-color:#4a2a2a;background:#1a1214}
@media (max-width:900px){.board{grid-template-columns:repeat(3,minmax(160px,1fr))}.find{width:100%}.find input{flex:1;min-width:0}}
"#;

fn status_badge(status: &str) -> String {
    format!(
        "<span class=\"status {}\">{}</span>",
        escape(status),
        escape(if status.is_empty() { "unknown" } else { status })
    )
}

fn tile(key: &str, value: String, class: &str) -> String {
    format!(
        "<div class=\"tile\"><div class=\"k\">{}</div><div class=\"v {class}\">{value}</div></div>",
        escape(key)
    )
}

/// The dashboard: summary, a small burndown, the board, and the busiest projects.
pub fn dashboard(
    book: &Book,
    project: Option<&str>,
    query: &str,
    days: i64,
    stale_days: i64,
) -> String {
    let chrome = Chrome {
        book,
        project,
        query,
        page: Page::Dashboard,
    };
    let summary = Summary::from(book, project, stale_days, book.now);
    let tiles = [
        tile("Issues", thousands(summary.total), ""),
        tile("Ready", thousands(summary.ready), "good"),
        tile("In progress", thousands(summary.in_progress), "accent"),
        tile("Review", thousands(summary.review), ""),
        tile("Blocked", thousands(summary.blocked), "bad"),
        tile("Stale", thousands(summary.stale), "warn"),
        tile("Done", format!("{:.1}%", summary.percent_done), "good"),
    ]
    .concat();

    let selected: Vec<Issue> = book
        .issues
        .iter()
        .filter(|issue| model::matches(issue, query, project))
        .cloned()
        .collect();
    let burn = model::burndown(&selected, days, book.now);
    let chart = burndown_chart(&burn, days);

    let columns = model::columns(&book.issues, query, project);
    let board = board_html(&columns, project, 6);

    let projects = model::projects(book, stale_days, book.now);
    let mut project_rows = String::new();
    for stat in projects.iter().take(8) {
        project_rows.push_str(&project_row(stat));
    }
    let project_table = if project_rows.is_empty() {
        "<p class=\"empty\">no projects yet</p>".to_string()
    } else {
        format!(
            "<table><thead><tr><th>Project</th><th class=\"num\">Total</th><th class=\"num\">Open</th><th class=\"num\">WIP</th><th class=\"num\">Review</th><th class=\"num\">Blocked</th><th class=\"num\">Ready</th><th class=\"num\">Closed</th><th class=\"num\">Stale</th><th>Progress</th></tr></thead><tbody>{project_rows}</tbody></table>"
        )
    };

    let body = format!(
        r#"<section class="tiles">{tiles}</section>
<section class="panel">
  <h2>Burndown · last {days} days</h2>
  <p class="hint">{scope} in scope · {done} closed · {remaining} remaining · {rate}</p>
  {chart}
</section>
<section class="panel">
  <h2>Board</h2>
  <p class="hint">Every issue by status, in the tracker's own order.{filtered}</p>
  {board}
</section>
<section class="panel">
  <h2>Projects</h2>
  <p class="hint">Ready follows the same rule as <code>mb ready</code>: open, past grace, unclaimed, every parent closed.</p>
  {project_table}
</section>"#,
        days = days,
        scope = thousands(burn.scope),
        done = thousands(burn.done),
        remaining = thousands(burn.remaining),
        rate = burn_rate(&burn),
        filtered = if query.is_empty() {
            String::new()
        } else {
            format!(" Filtered by <code>{}</code>.", escape(query))
        },
    );
    let sub = format!(
        "{} issues across {} projects · {} ready · {} stale",
        thousands(summary.total),
        thousands(model::project_slugs(book).len() as i64),
        thousands(summary.ready),
        thousands(summary.stale)
    );
    chrome.wrap("Marbles", &escape(&sub), &body)
}

/// The board on its own, at full width.
pub fn board_page(book: &Book, project: Option<&str>, query: &str, limit: usize) -> String {
    let chrome = Chrome {
        book,
        project,
        query,
        page: Page::Board,
    };
    let columns = model::columns(&book.issues, query, project);
    let shown: usize = columns.iter().map(Column::count).sum();
    let body = format!(
        "<section class=\"panel\"><p class=\"hint\">{} cards{}</p>{}</section>",
        thousands(shown as i64),
        if query.is_empty() {
            String::new()
        } else {
            format!(" matching <code>{}</code>", escape(query))
        },
        board_html(&columns, project, limit)
    );
    chrome.wrap(
        "Board",
        "Every issue by status; an unknown status gets a column rather than disappearing.",
        &body,
    )
}

/// The burndown on its own, with the day table.
pub fn burndown_page(book: &Book, project: Option<&str>, query: &str, days: i64) -> String {
    let chrome = Chrome {
        book,
        project,
        query,
        page: Page::Burndown,
    };
    let selected: Vec<Issue> = book
        .issues
        .iter()
        .filter(|issue| model::matches(issue, query, project))
        .cloned()
        .collect();
    let burn = model::burndown(&selected, days, book.now);
    let mut rows = String::new();
    for day in burn.days.iter().rev().take(days.min(31) as usize) {
        let _ = write!(
            rows,
            "<tr><td>{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
            escape(&date_label(day.at)),
            thousands(day.scope),
            thousands(day.done),
            thousands(day.remaining),
            thousands(day.ideal)
        );
    }
    let body = format!(
        r#"<section class="panel">
  <h2>Remaining against the ideal</h2>
  <p class="hint">{scope} in scope · {done} closed · {remaining} remaining · {rate}</p>
  {chart}
</section>
<section class="panel">
  <h2>Days</h2>
  <table><thead><tr><th>Day</th><th class="num">Scope</th><th class="num">Done</th><th class="num">Remaining</th><th class="num">Ideal</th></tr></thead>
  <tbody>{rows}</tbody></table>
</section>"#,
        scope = thousands(burn.scope),
        done = thousands(burn.done),
        remaining = thousands(burn.remaining),
        rate = burn_rate(&burn),
        chart = burndown_chart(&burn, days),
    );
    chrome.wrap(
        "Burndown",
        "Work that existed by each day's end, against an even burn of the same scope.",
        &body,
    )
}

/// The project rollup.
pub fn projects_page(book: &Book, project: Option<&str>, query: &str, stale_days: i64) -> String {
    let chrome = Chrome {
        book,
        project,
        query,
        page: Page::Projects,
    };
    let rows = model::projects(book, stale_days, book.now);
    let mut body = String::new();
    for stat in &rows {
        body.push_str(&project_row(stat));
    }
    let table = if body.is_empty() {
        "<p class=\"empty\">no projects yet</p>".to_string()
    } else {
        format!(
            "<table><thead><tr><th>Project</th><th class=\"num\">Total</th><th class=\"num\">Open</th><th class=\"num\">WIP</th><th class=\"num\">Review</th><th class=\"num\">Blocked</th><th class=\"num\">Ready</th><th class=\"num\">Closed</th><th class=\"num\">Stale</th><th>Progress</th></tr></thead><tbody>{body}</tbody></table>"
        )
    };
    chrome.wrap(
        "Projects",
        "One row per project in the book. Ready is claimable work; stale is open work untouched for the window.",
        &format!("<section class=\"panel\">{table}</section>"),
    )
}

/// Plans: one row per `metadata.plan` value.
pub fn plans_page(book: &Book, project: Option<&str>, query: &str) -> String {
    let chrome = Chrome {
        book,
        project,
        query,
        page: Page::Plans,
    };
    let plans = model::plans(book, book.now);
    let mut rows = String::new();
    for PlanStat {
        name,
        nodes,
        closed,
        ready,
        blocked,
    } in &plans
    {
        let _ = write!(
            rows,
            "<tr><td><a href=\"{}\">{}</a></td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td><div class=\"bar\"><i style=\"width:{:.1}%\"></i></div></td></tr>",
            link("/plan", None, &[("name", name)]),
            escape(name),
            thousands(*nodes),
            thousands(*closed),
            thousands(*ready),
            thousands(*blocked),
            if *nodes > 0 {
                *closed as f64 / *nodes as f64 * 100.0
            } else {
                0.0
            }
        );
    }
    let body = if rows.is_empty() {
        "<section class=\"panel\"><p class=\"empty\">no plans in this book</p></section>"
            .to_string()
    } else {
        format!(
            "<section class=\"panel\"><table><thead><tr><th>Plan</th><th class=\"num\">Nodes</th><th class=\"num\">Closed</th><th class=\"num\">Ready</th><th class=\"num\">Blocked</th><th>Progress</th></tr></thead><tbody>{rows}</tbody></table></section>"
        )
    };
    chrome.wrap(
        "Plans",
        "A plan is a metadata predicate over ordinary issues, not a second store.",
        &body,
    )
}

/// One plan and its nodes.
pub fn plan_page(book: &Book, name: &str, project: Option<&str>, query: &str) -> String {
    let chrome = Chrome {
        book,
        project,
        query,
        page: Page::Plans,
    };
    let nodes = model::plan_nodes(book, name);
    if nodes.is_empty() {
        return error_page(book, 404, &format!("no plan named {name}"));
    }
    let by_id = book.index();
    let mut rows = String::new();
    for node in &nodes {
        let reasons = model::blocking_in(book, &by_id, node, book.now);
        let wait = if reasons.is_empty() {
            "<span class=\"ready\">ready</span>".to_string()
        } else {
            format!(
                "<span class=\"notready\">{}</span>",
                escape(&reasons.join("; "))
            )
        };
        let mut deps = Vec::new();
        for parent in book
            .parents
            .get(&node.id)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let status = by_id
                .get(parent.as_str())
                .map(|issue| issue.status.as_str())
                .unwrap_or("missing");
            deps.push(format!("{} [{status}]", escape(parent)));
        }
        let _ = write!(
            rows,
            "<tr><td><code>{}</code></td><td><a href=\"{}\">{}</a></td><td>{}</td><td class=\"num\">p{}</td><td>{wait}</td><td>{}</td></tr>",
            escape(&node.id),
            link("/issue", None, &[("id", &node.id)]),
            escape(&node.title),
            status_badge(&node.status),
            node.priority,
            deps.join("<br>")
        );
    }
    let body = format!(
        "<section class=\"panel\"><h2>{}</h2><p class=\"hint\">{} node(s)</p><table><thead><tr><th>Id</th><th>Title</th><th>Status</th><th class=\"num\">Priority</th><th>Readiness</th><th>Waits on</th></tr></thead><tbody>{rows}</tbody></table></section>",
        escape(name),
        nodes.len()
    );
    chrome.wrap(
        &format!("Plan · {name}"),
        "Nodes are ordinary issues carrying plan metadata.",
        &body,
    )
}

/// One issue with its edges, evidence and metadata.
pub fn issue_page(book: &Book, id: &str, project: Option<&str>, query: &str) -> String {
    let chrome = Chrome {
        book,
        project,
        query,
        page: Page::Board,
    };
    let Some(issue) = book.issues.iter().find(|issue| issue.id == id) else {
        return error_page(book, 404, &format!("no issue {id}"));
    };
    let by_id = book.index();
    let reasons = model::blocking_in(book, &by_id, issue, book.now);
    let readiness = if reasons.is_empty() {
        "<span class=\"ready\">ready to claim</span>".to_string()
    } else {
        format!(
            "<span class=\"notready\">{}</span>",
            escape(&reasons.join("; "))
        )
    };

    let mut waits = String::new();
    for parent in book
        .parents
        .get(&issue.id)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let (title, status) = by_id
            .get(parent.as_str())
            .map(|issue| (issue.title.as_str(), issue.status.as_str()))
            .unwrap_or(("(missing from the book)", "missing"));
        let _ = write!(
            waits,
            "<li><a href=\"{}\"><code>{}</code></a> {} {}</li>",
            link("/issue", None, &[("id", parent)]),
            escape(parent),
            status_badge(status),
            escape(title)
        );
    }
    if waits.is_empty() {
        waits.push_str("<li class=\"empty\">nothing</li>");
    }
    let mut blocks = String::new();
    for child in book
        .children
        .get(&issue.id)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let (title, status) = by_id
            .get(child.as_str())
            .map(|issue| (issue.title.as_str(), issue.status.as_str()))
            .unwrap_or(("(missing from the book)", "missing"));
        let _ = write!(
            blocks,
            "<li><a href=\"{}\"><code>{}</code></a> {} {}</li>",
            link("/issue", None, &[("id", child)]),
            escape(child),
            status_badge(status),
            escape(title)
        );
    }
    if blocks.is_empty() {
        blocks.push_str("<li class=\"empty\">nothing</li>");
    }

    let mut evidence = String::new();
    for item in &issue.evidence {
        let _ = write!(
            evidence,
            "<li><span class=\"chip label\">{}</span> <code>{}</code></li>",
            escape(&item.kind),
            escape(&item.value)
        );
    }
    if evidence.is_empty() {
        evidence.push_str("<li class=\"empty\">none</li>");
    }

    let metadata = if issue.metadata.is_null()
        || issue.metadata.as_object().is_some_and(|map| map.is_empty())
    {
        "<span class=\"empty\">none</span>".to_string()
    } else {
        format!(
            "<pre><code>{}</code></pre>",
            escape(&serde_json::to_string_pretty(&issue.metadata).unwrap_or_default())
        )
    };
    let plan = plan_name(issue)
        .map(|name| {
            format!(
                "<a href=\"{}\">{}</a>",
                link("/plan", None, &[("name", name)]),
                escape(name)
            )
        })
        .unwrap_or_else(|| "<span class=\"empty\">none</span>".to_string());

    let body = format!(
        r#"<div class="grid2">
<section class="panel">
  <h2>{} {status_badge}</h2>
  <p class="hint"><code>{}</code> · {} · p{} · {}</p>
  <p>{}</p>
  <dl class="kv">
    <dt>Project</dt><dd>{}</dd>
    <dt>Type</dt><dd>{}</dd>
    <dt>Readiness</dt><dd>{readiness}</dd>
    <dt>Holder</dt><dd>{}</dd>
    <dt>Created</dt><dd>{}</dd>
    <dt>Updated</dt><dd>{}</dd>
    <dt>Closed</dt><dd>{}</dd>
    <dt>Available</dt><dd>{}</dd>
    <dt>Lease expires</dt><dd>{}</dd>
    <dt>Plan</dt><dd>{plan}</dd>
    <dt>External ref</dt><dd>{}</dd>
    <dt>Labels</dt><dd>{}</dd>
  </dl>
</section>
<section class="panel">
  <h2>Waits on</h2><ul class="plain">{waits}</ul>
  <h2 style="margin-top:16px">Blocks</h2><ul class="plain">{blocks}</ul>
  <h2 style="margin-top:16px">Evidence</h2><ul class="plain">{evidence}</ul>
  <h2 style="margin-top:16px">Metadata</h2>{metadata}
</section>
</div>"#,
        escape(&issue.title),
        escape(&issue.id),
        escape(&issue.project),
        issue.priority,
        escape(&issue.issue_type),
        escape(&issue.description),
        escape(&issue.project),
        escape(&issue.issue_type),
        escape(issue.assignee.as_deref().unwrap_or("unclaimed")),
        escape(&stamp(issue.created_at, book.now)),
        escape(&stamp(issue.updated_at, book.now)),
        escape(
            &issue
                .closed_at
                .map(|at| stamp(at, book.now))
                .unwrap_or_else(|| "—".into())
        ),
        escape(
            &issue
                .available_at
                .map(|at| stamp(at, book.now))
                .unwrap_or_else(|| "—".into())
        ),
        escape(
            &issue
                .expires_at
                .map(|at| stamp(at, book.now))
                .unwrap_or_else(|| "—".into())
        ),
        escape(issue.external_ref.as_deref().unwrap_or("—")),
        if issue.labels.is_empty() {
            "<span class=\"empty\">none</span>".to_string()
        } else {
            issue
                .labels
                .iter()
                .map(|label| format!("<span class=\"chip label\">{}</span>", escape(label)))
                .collect::<Vec<_>>()
                .join(" ")
        },
        status_badge = status_badge(&issue.status),
    );
    chrome.wrap(&format!("{} — {}", issue.id, issue.title), "", &body)
}

/// A page for a failure. The same chrome, so a missing issue is still navigable.
pub fn error_page(book: &Book, status: u16, message: &str) -> String {
    let chrome = Chrome {
        book,
        project: None,
        query: "",
        page: Page::Dashboard,
    };
    chrome.wrap(
        &format!("{status}"),
        "",
        &format!(
            "<section class=\"panel error\"><h2>Not found</h2><p>{}</p></section>",
            escape(message)
        ),
    )
}

fn stamp(at: i64, now: i64) -> String {
    if at == 0 {
        return "—".into();
    }
    let days = (now - at) / 86_400;
    let absolute = date_label(at);
    if at > now {
        format!("{absolute} (scheduled)")
    } else if days <= 0 {
        format!("{absolute} (today)")
    } else {
        format!("{absolute} ({days}d ago)")
    }
}

fn burn_rate(burn: &Burndown) -> String {
    match (burn.burn_per_day, burn.projected_days_to_zero) {
        (Some(rate), Some(days)) => format!("burn {rate:.2}/day · {days:.0}d to zero"),
        (Some(rate), None) => format!("burn {rate:.2}/day"),
        _ => "burn unknown (no closes in the window)".to_string(),
    }
}

fn project_row(stat: &ProjectStat) -> String {
    let percent = if stat.total > 0 {
        stat.closed as f64 / stat.total as f64 * 100.0
    } else {
        0.0
    };
    format!(
        "<tr><td><a href=\"{}\">{}</a></td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td><div class=\"bar\"><i style=\"width:{percent:.1}%\"></i></div></td></tr>",
        link("/board", Some(&stat.slug), &[]),
        escape(&stat.slug),
        thousands(stat.total),
        thousands(stat.open),
        thousands(stat.in_progress),
        thousands(stat.review),
        thousands(stat.blocked),
        thousands(stat.ready),
        thousands(stat.closed),
        thousands(stat.stale),
    )
}

fn board_html(columns: &[Column], project: Option<&str>, limit: usize) -> String {
    let mut out = String::from("<div class=\"board\">");
    for column in columns {
        let _ = write!(
            out,
            "<div class=\"col\"><h3>{}<span class=\"n\">{}</span></h3>",
            escape(column.title),
            thousands(column.count() as i64)
        );
        if column.cards.is_empty() {
            out.push_str("<p class=\"empty\">none</p>");
        }
        for card in column.cards.iter().take(limit) {
            let _ = write!(
                out,
                "<a class=\"card\" href=\"{}\"><span class=\"id\">{}</span><span class=\"title\">{}</span><span class=\"chips\"><span class=\"chip{}\">p{}</span>{}</span></a>",
                link("/issue", project, &[("id", &card.id)]),
                escape(&card.id),
                escape(&card.title),
                if card.priority <= 1 { " p1" } else { "" },
                card.priority,
                chips(card),
            );
        }
        let hidden = column.count().saturating_sub(limit);
        if hidden > 0 {
            let _ = write!(
                out,
                "<p class=\"empty\">… {} more (narrow with a project or search)</p>",
                thousands(hidden as i64)
            );
        }
        out.push_str("</div>");
    }
    out.push_str("</div>");
    out
}

fn chips(card: &model::Card) -> String {
    let mut out = String::new();
    if !card.project.is_empty() {
        let _ = write!(out, "<span class=\"chip\">{}</span>", escape(&card.project));
    }
    if let Some(who) = &card.assignee {
        let _ = write!(out, "<span class=\"chip who\">{}</span>", escape(who));
    }
    for label in card.labels.iter().take(3) {
        let _ = write!(out, "<span class=\"chip label\">{}</span>", escape(label));
    }
    out
}

/// The burndown as an inline SVG. No chart library, no canvas, no script: the
/// browser receives the lines already computed.
fn burndown_chart(burn: &Burndown, days: i64) -> String {
    const W: f64 = 980.0;
    const H: f64 = 260.0;
    const LEFT: f64 = 46.0;
    const RIGHT: f64 = 14.0;
    const TOP: f64 = 12.0;
    const BOTTOM: f64 = 26.0;
    let peak = burn.peak().max(1) as f64;
    let plot_w = W - LEFT - RIGHT;
    let plot_h = H - TOP - BOTTOM;
    let count = burn.days.len().max(2) - 1;
    let x = |index: usize| LEFT + (index as f64 / count as f64) * plot_w;
    let y = |value: i64| TOP + (1.0 - value as f64 / peak) * plot_h;

    let line = |get: &dyn Fn(&model::Day) -> i64, class: &str, dash: &str| -> String {
        let mut points = String::new();
        for (index, day) in burn.days.iter().enumerate() {
            let _ = write!(points, "{:.1},{:.1} ", x(index), y(get(day)));
        }
        format!(
            "<polyline class=\"{class}\" fill=\"none\" stroke-width=\"2\"{dash} points=\"{}\"/>",
            points.trim_end()
        )
    };

    let mut grid = String::new();
    for step in 0..=2 {
        let value = (peak * step as f64 / 2.0).round() as i64;
        let at = y(value);
        let _ = write!(
            grid,
            "<line x1=\"{LEFT}\" y1=\"{at:.1}\" x2=\"{:.1}\" y2=\"{at:.1}\" stroke=\"#222e3a\" stroke-width=\"1\"/><text x=\"{:.1}\" y=\"{:.1}\" fill=\"#5d6b7a\" font-size=\"10\" text-anchor=\"end\">{}</text>",
            W - RIGHT,
            LEFT - 6.0,
            at + 3.0,
            thousands(value)
        );
    }
    let first = burn.days.first().map(|day| day.at).unwrap_or(0);
    let last = burn.days.last().map(|day| day.at).unwrap_or(0);
    let labels = format!(
        "<text x=\"{LEFT}\" y=\"{:.1}\" fill=\"#5d6b7a\" font-size=\"10\" text-anchor=\"start\">{}</text><text x=\"{:.1}\" y=\"{:.1}\" fill=\"#5d6b7a\" font-size=\"10\" text-anchor=\"end\">{}</text>",
        H - 8.0,
        escape(&date_label(first)),
        W - RIGHT,
        H - 8.0,
        escape(&date_label(last))
    );
    let area = {
        let mut points = format!("{:.1},{:.1} ", LEFT, TOP + plot_h);
        for (index, day) in burn.days.iter().enumerate() {
            let _ = write!(points, "{:.1},{:.1} ", x(index), y(day.remaining));
        }
        let _ = write!(points, "{:.1},{:.1}", W - RIGHT, TOP + plot_h);
        format!("<polygon fill=\"#6ea8fe14\" points=\"{}\"/>", points)
    };
    format!(
        r#"<svg class="chart" viewBox="0 0 {W} {H}" role="img" aria-label="Burndown over {days} days">
<style>.scope{{stroke:#5d6b7a}}.remaining{{stroke:#6ea8fe}}.ideal{{stroke:#c4a7ff;stroke-dasharray:5 5;opacity:.75}}</style>
{grid}{labels}{area}{scope}{remaining}{ideal}
</svg>
<div class="legend"><span><i style="background:#5d6b7a"></i>scope</span><span><i style="background:#6ea8fe"></i>remaining</span><span><i style="background:#c4a7ff"></i>ideal</span></div>"#,
        grid = grid,
        labels = labels,
        area = area,
        scope = line(&|day| day.scope, "scope", ""),
        remaining = line(&|day| day.remaining, "remaining", ""),
        ideal = line(&|day| day.ideal, "ideal", ""),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_escaped_everywhere_it_can_come_from_a_person() {
        assert_eq!(escape("<script>"), "&lt;script&gt;");
        assert_eq!(escape("a & b"), "a &amp; b");
        assert_eq!(escape("\"x\" 'y'"), "&quot;x&quot; &#39;y&#39;");
    }

    #[test]
    fn query_values_are_percent_encoded() {
        assert_eq!(url_encode("personal/avery"), "personal%2Favery");
        assert_eq!(url_encode("a b"), "a+b");
    }
}
