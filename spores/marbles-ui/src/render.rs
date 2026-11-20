//! The server-rendered pages.
//!
//! One function per route, each returning a complete HTML document. There is no
//! client-side state, no JSON fetch and no framework: the guest reads the book,
//! computes the projections in [`crate::model`], and writes the page. What the
//! browser receives is what the book said at that moment.
//!
//! Markup is built with [`maud`], the same macro `spore-mail` uses. That is not
//! only a style choice: `html!` escapes every interpolated string by default, so
//! an issue title or label containing `<` cannot become an injection by being
//! pasted into the page. The one deliberate exception is the burndown SVG, which
//! is generated from numbers here and wrapped in [`PreEscaped`].

use maud::{html, Markup, PreEscaped, DOCTYPE};

use crate::model::{
    self, date_label, plan_name, Book, Burndown, Column, Issue, PlanStat, ProjectStat, Summary,
};

const CSS: &str = include_str!("style.css");

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
            _ => out.push_str(&format!("%{byte:02X}")),
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
    fn options(&self) -> Markup {
        html! {
            option value="" { "All projects" }
            @for slug in model::project_slugs(self.book) {
                @if self.project == Some(slug.as_str()) {
                    option value=(slug) selected { (slug) }
                } @else {
                    option value=(slug) { (slug) }
                }
            }
        }
    }

    fn nav(&self) -> Markup {
        html! {
            @for page in [Page::Dashboard, Page::Board, Page::Burndown, Page::Projects, Page::Plans] {
                a href=(link(page.path(), self.project, &[]))
                  class=(if page == self.page { "on" } else { "" }) {
                    (page.title())
                }
            }
        }
    }

    /// Render `body` inside the document.
    pub fn wrap(&self, heading: &str, sub: &str, body: Markup) -> Markup {
        let title = if heading.is_empty() || heading == "Marbles" {
            "Marbles".to_string()
        } else {
            format!("{heading} — Marbles")
        };
        html! {
            (DOCTYPE)
            html lang="en" {
                head {
                    meta charset="utf-8";
                    meta name="viewport" content="width=device-width, initial-scale=1";
                    title { (title) }
                    style { (PreEscaped(CSS)) }
                }
                body {
                    header class="top" {
                        a class="brand" href=(link("/", self.project, &[])) {
                            span class="mark" { "M" }
                            " Marbles"
                        }
                        nav { (self.nav()) }
                        form class="find" method="get" action=(link("/board", None, &[])) {
                            input type="search" name="q" value=(self.query)
                                placeholder="search id, title, label, holder"
                                aria-label="Search issues";
                            select name="project" aria-label="Project" { (self.options()) }
                            button type="submit" { "Search" }
                        }
                    }
                    main {
                        div class="head" {
                            h1 { (heading) }
                            p class="sub" { (sub) }
                        }
                        (body)
                    }
                    footer {
                        span { "read-only view of " code { "soil:records" } }
                        span { "now = newest record in the book · " (date_label(self.book.now)) }
                    }
                }
            }
        }
    }
}

fn status_badge(status: &str) -> Markup {
    let label = if status.is_empty() { "unknown" } else { status };
    html! { span class=(format!("status {status}")) { (label) } }
}

fn tile(key: &str, value: String, class: &str) -> Markup {
    html! {
        div class="tile" {
            div class="k" { (key) }
            div class=(format!("v {class}")) { (value) }
        }
    }
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
    let selected: Vec<Issue> = book
        .issues
        .iter()
        .filter(|issue| model::matches(issue, query, project))
        .cloned()
        .collect();
    let burn = model::burndown(&selected, days, book.now);
    let columns = model::columns(&book.issues, query, project);
    let projects = model::projects(book, stale_days, book.now);
    let sub = format!(
        "{} issues across {} projects · {} ready · {} stale",
        thousands(summary.total),
        thousands(model::project_slugs(book).len() as i64),
        thousands(summary.ready),
        thousands(summary.stale)
    );
    let filtered = if query.is_empty() {
        String::new()
    } else {
        format!(" Filtered by {query}.")
    };
    let body = html! {
        section class="tiles" {
            (tile("Issues", thousands(summary.total), ""))
            (tile("Ready", thousands(summary.ready), "good"))
            (tile("In progress", thousands(summary.in_progress), "accent"))
            (tile("Review", thousands(summary.review), ""))
            (tile("Blocked", thousands(summary.blocked), "bad"))
            (tile("Stale", thousands(summary.stale), "warn"))
            (tile("Done", format!("{:.1}%", summary.percent_done), "good"))
        }
        section class="panel" {
            h2 { "Burndown · last " (days) " days" }
            p class="hint" {
                (thousands(burn.scope)) " in scope · "
                (thousands(burn.done)) " closed · "
                (thousands(burn.remaining)) " remaining · "
                (burn_rate(&burn))
            }
            (burndown_chart(&burn, days))
        }
        section class="panel" {
            h2 { "Board" }
            p class="hint" { "Every issue by status, in the tracker's own order." (filtered) }
            (board_html(&columns, project, query, 6))
        }
        section class="panel" {
            h2 { "Projects" }
            p class="hint" {
                "Ready follows the same rule as " code { "mb ready" }
                ": open, past grace, unclaimed, every parent closed."
            }
            @if projects.is_empty() {
                p class="empty" { "no projects yet" }
            } @else {
                table {
                    thead {
                        tr {
                            th { "Project" }
                            th class="num" { "Total" }
                            th class="num" { "Open" }
                            th class="num" { "WIP" }
                            th class="num" { "Review" }
                            th class="num" { "Blocked" }
                            th class="num" { "Ready" }
                            th class="num" { "Closed" }
                            th class="num" { "Stale" }
                            th { "Progress" }
                        }
                    }
                    tbody {
                        @for stat in projects.iter().take(8) {
                            (project_row(stat))
                        }
                    }
                }
            }
        }
    };
    chrome.wrap("Marbles", &sub, body).into_string()
}

/// The board on its own, at full width.
///
/// Without `status` this is the six-column overview, capped per column. With
/// `status` it is that one status, paginated: a column with 2,400 cards is not
/// a page anyone can read, and rendering all of them is the largest thing this
/// Spore can be asked to do.
pub fn board_page(
    book: &Book,
    project: Option<&str>,
    query: &str,
    limit: usize,
    status: Option<&str>,
    page: usize,
) -> String {
    let chrome = Chrome {
        book,
        project,
        query,
        page: Page::Board,
    };
    let columns = model::columns(&book.issues, query, project);
    let filtered = if query.is_empty() {
        String::new()
    } else {
        format!(" matching {query}")
    };
    let body = match status {
        Some(status) => {
            let column = columns.iter().find(|column| column.status == status);
            let cards = column.map(|column| column.cards.as_slice()).unwrap_or(&[]);
            let total = cards.len();
            let pages = total.div_ceil(limit).max(1);
            let page = page.clamp(1, pages);
            let start = (page - 1).saturating_mul(limit);
            let shown: Vec<&model::Card> = cards.iter().skip(start).take(limit).collect();
            html! {
                section class="panel" {
                    div class="boardhead" {
                        div {
                            h2 {
                                @if let Some(column) = column {
                                    (column.title)
                                } @else {
                                    (status)
                                }
                            }
                            p class="hint" {
                                (thousands(total as i64)) " cards" (filtered)
                                " · showing " (thousands(shown.len() as i64))
                            }
                        }
                        (pager(project, query, status, limit, page, pages))
                    }
                    @if shown.is_empty() {
                        p class="empty" { "no cards in this column" }
                    } @else {
                        div class="cards-grid" {
                            @for card in &shown {
                                (card_html(card, project))
                            }
                        }
                        (pager(project, query, status, limit, page, pages))
                    }
                }
            }
        }
        None => {
            let shown: usize = columns.iter().map(Column::count).sum();
            html! {
                section class="panel" {
                    p class="hint" {
                        (thousands(shown as i64)) " cards" (filtered)
                        " · each column shows the first 12; open a status to page through it"
                    }
                    (board_html(&columns, project, query, 12))
                }
            }
        }
    };
    chrome
        .wrap(
            "Board",
            "Every issue by status; an unknown status gets a column rather than disappearing.",
            body,
        )
        .into_string()
}

fn pager(
    project: Option<&str>,
    query: &str,
    status: &str,
    limit: usize,
    page: usize,
    pages: usize,
) -> Markup {
    if pages <= 1 {
        return html! {};
    }
    let limit = limit.to_string();
    let href = |target: usize| {
        link(
            "/board",
            project,
            &[
                ("q", query),
                ("status", status),
                ("limit", &limit),
                ("page", &target.to_string()),
            ],
        )
    };
    html! {
        nav class="pager" {
            @if page > 1 {
                a href=(href(page - 1)) { "← prev" }
            } @else {
                span class="off" { "← prev" }
            }
            span class="pages" { "page " (page) " / " (pages) }
            @if page < pages {
                a href=(href(page + 1)) { "next →" }
            } @else {
                span class="off" { "next →" }
            }
        }
    }
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
    let body = html! {
        section class="panel" {
            h2 { "Remaining against the ideal" }
            p class="hint" {
                (thousands(burn.scope)) " in scope · "
                (thousands(burn.done)) " closed · "
                (thousands(burn.remaining)) " remaining · "
                (burn_rate(&burn))
            }
            (burndown_chart(&burn, days))
        }
        section class="panel" {
            h2 { "Days" }
            table {
                thead {
                    tr {
                        th { "Day" }
                        th class="num" { "Scope" }
                        th class="num" { "Done" }
                        th class="num" { "Remaining" }
                        th class="num" { "Ideal" }
                    }
                }
                tbody {
                    @for day in burn.days.iter().rev().take(days.min(31) as usize) {
                        tr {
                            td { (date_label(day.at)) }
                            td class="num" { (thousands(day.scope)) }
                            td class="num" { (thousands(day.done)) }
                            td class="num" { (thousands(day.remaining)) }
                            td class="num" { (thousands(day.ideal)) }
                        }
                    }
                }
            }
        }
    };
    chrome
        .wrap(
            "Burndown",
            "Work that existed by each day's end, against an even burn of the same scope.",
            body,
        )
        .into_string()
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
    let body = html! {
        section class="panel" {
            @if rows.is_empty() {
                p class="empty" { "no projects yet" }
            } @else {
                table {
                    thead {
                        tr {
                            th { "Project" }
                            th class="num" { "Total" }
                            th class="num" { "Open" }
                            th class="num" { "WIP" }
                            th class="num" { "Review" }
                            th class="num" { "Blocked" }
                            th class="num" { "Ready" }
                            th class="num" { "Closed" }
                            th class="num" { "Stale" }
                            th { "Progress" }
                        }
                    }
                    tbody {
                        @for stat in &rows {
                            (project_row(stat))
                        }
                    }
                }
            }
        }
    };
    chrome
        .wrap(
            "Projects",
            "One row per project in the book. Ready is claimable work; stale is open work untouched for the window.",
            body,
        )
        .into_string()
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
    let body = html! {
        section class="panel" {
            @if plans.is_empty() {
                p class="empty" { "no plans in this book" }
            } @else {
                table {
                    thead {
                        tr {
                            th { "Plan" }
                            th class="num" { "Nodes" }
                            th class="num" { "Closed" }
                            th class="num" { "Ready" }
                            th class="num" { "Blocked" }
                            th { "Progress" }
                        }
                    }
                    tbody {
                        @for PlanStat { name, nodes, closed, ready, blocked } in &plans {
                            tr {
                                td { a href=(link("/plan", None, &[("name", name)])) { (name) } }
                                td class="num" { (thousands(*nodes)) }
                                td class="num" { (thousands(*closed)) }
                                td class="num" { (thousands(*ready)) }
                                td class="num" { (thousands(*blocked)) }
                                td {
                                    div class="bar" {
                                        i style=(format!("width:{:.1}%", percent(*closed, *nodes))) {}
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    };
    chrome
        .wrap(
            "Plans",
            "A plan is a metadata predicate over ordinary issues, not a second store.",
            body,
        )
        .into_string()
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
    let body = html! {
        section class="panel" {
            h2 { (name) }
            p class="hint" { (nodes.len()) " node(s)" }
            table {
                thead {
                    tr {
                        th { "Id" }
                        th { "Title" }
                        th { "Status" }
                        th class="num" { "Priority" }
                        th { "Readiness" }
                        th { "Waits on" }
                    }
                }
                tbody {
                    @for node in &nodes {
                        (plan_row(book, &by_id, node))
                    }
                }
            }
        }
    };
    chrome
        .wrap(
            &format!("Plan · {name}"),
            "Nodes are ordinary issues carrying plan metadata.",
            body,
        )
        .into_string()
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
    let parents = edges(book, &by_id, &issue.id, true);
    let children = edges(book, &by_id, &issue.id, false);
    let labels = if issue.labels.is_empty() {
        html! { span class="empty" { "none" } }
    } else {
        html! {
            @for label in &issue.labels {
                span class="chip label" { (label) }
            }
        }
    };
    let plan = plan_name(issue).map_or_else(
        || html! { span class="empty" { "none" } },
        |name| html! { a href=(link("/plan", None, &[("name", name)])) { (name) } },
    );
    let metadata = if issue.metadata.is_null()
        || issue.metadata.as_object().is_some_and(|map| map.is_empty())
    {
        html! { span class="empty" { "none" } }
    } else {
        let pretty = serde_json::to_string_pretty(&issue.metadata).unwrap_or_default();
        html! { pre { code { (pretty) } } }
    };
    let body = html! {
        div class="grid2" {
            section class="panel" {
                h2 {
                    (issue.title) " " (status_badge(&issue.status))
                }
                p class="hint" {
                    code { (issue.id) } " · " (issue.project)
                    " · p" (issue.priority) " · " (issue.issue_type)
                }
                p { (issue.description) }
                dl class="kv" {
                    dt { "Project" } dd { (issue.project) }
                    dt { "Type" } dd { (issue.issue_type) }
                    dt { "Readiness" }
                    dd {
                        @if reasons.is_empty() {
                            span class="ready" { "ready to claim" }
                        } @else {
                            span class="notready" { (reasons.join("; ")) }
                        }
                    }
                    dt { "Holder" }
                    dd { (issue.assignee.as_deref().unwrap_or("unclaimed")) }
                    dt { "Created" } dd { (stamp(issue.created_at, book.now)) }
                    dt { "Updated" } dd { (stamp(issue.updated_at, book.now)) }
                    dt { "Closed" }
                    dd { (issue.closed_at.map(|at| stamp(at, book.now)).unwrap_or_else(|| "—".into())) }
                    dt { "Available" }
                    dd { (issue.available_at.map(|at| stamp(at, book.now)).unwrap_or_else(|| "—".into())) }
                    dt { "Lease expires" }
                    dd { (issue.expires_at.map(|at| stamp(at, book.now)).unwrap_or_else(|| "—".into())) }
                    dt { "Plan" } dd { (plan) }
                    dt { "External ref" }
                    dd { (issue.external_ref.as_deref().unwrap_or("—")) }
                    dt { "Labels" } dd { (labels) }
                }
            }
            section class="panel" {
                h2 { "Waits on" }
                ul class="plain" {
                    @if parents.is_empty() {
                        li class="empty" { "nothing" }
                    }
                    @for (id, title, status) in &parents {
                        (edge_line(id, title, status))
                    }
                }
                h2 style="margin-top:16px" { "Blocks" }
                ul class="plain" {
                    @if children.is_empty() {
                        li class="empty" { "nothing" }
                    }
                    @for (id, title, status) in &children {
                        (edge_line(id, title, status))
                    }
                }
                h2 style="margin-top:16px" { "Evidence" }
                ul class="plain" {
                    @if issue.evidence.is_empty() {
                        li class="empty" { "none" }
                    }
                    @for item in &issue.evidence {
                        li {
                            span class="chip label" { (item.kind) }
                            " " code { (item.value) }
                        }
                    }
                }
                h2 style="margin-top:16px" { "Metadata" }
                (metadata)
            }
        }
    };
    chrome
        .wrap(&format!("{} — {}", issue.id, issue.title), "", body)
        .into_string()
}

/// A page for a failure. The same chrome, so a missing issue is still navigable.
pub fn error_page(book: &Book, status: u16, message: &str) -> String {
    let chrome = Chrome {
        book,
        project: None,
        query: "",
        page: Page::Dashboard,
    };
    chrome
        .wrap(
            &status.to_string(),
            "",
            html! {
                section class="panel error" {
                    h2 { "Not found" }
                    p { (message) }
                }
            },
        )
        .into_string()
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

fn percent(part: i64, whole: i64) -> f64 {
    if whole > 0 {
        part as f64 / whole as f64 * 100.0
    } else {
        0.0
    }
}

fn project_row(stat: &ProjectStat) -> Markup {
    html! {
        tr {
            td { a href=(link("/board", Some(&stat.slug), &[])) { (stat.slug) } }
            td class="num" { (thousands(stat.total)) }
            td class="num" { (thousands(stat.open)) }
            td class="num" { (thousands(stat.in_progress)) }
            td class="num" { (thousands(stat.review)) }
            td class="num" { (thousands(stat.blocked)) }
            td class="num" { (thousands(stat.ready)) }
            td class="num" { (thousands(stat.closed)) }
            td class="num" { (thousands(stat.stale)) }
            td {
                div class="bar" {
                    i style=(format!("width:{:.1}%", percent(stat.closed, stat.total))) {}
                }
            }
        }
    }
}

fn plan_row(book: &Book, by_id: &std::collections::BTreeMap<&str, &Issue>, node: &Issue) -> Markup {
    let reasons = model::blocking_in(book, by_id, node, book.now);
    let parents: Vec<(String, String, String)> = book
        .parents
        .get(&node.id)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|parent| {
            by_id.get(parent.as_str()).map_or_else(
                || (parent.clone(), "(missing)".into(), "missing".into()),
                |issue| (parent.clone(), issue.title.clone(), issue.status.clone()),
            )
        })
        .collect();
    html! {
        tr {
            td { code { (node.id) } }
            td { a href=(link("/issue", None, &[("id", &node.id)])) { (node.title) } }
            td { (status_badge(&node.status)) }
            td class="num" { "p" (node.priority) }
            td {
                @if reasons.is_empty() {
                    span class="ready" { "ready" }
                } @else {
                    span class="notready" { (reasons.join("; ")) }
                }
            }
            td {
                @for (parent, title, status) in &parents {
                    code { (parent) } " [" (status) "] " (title) br;
                }
            }
        }
    }
}

/// `(id, title, status)` for a node's parents (or children).
fn edges(
    book: &Book,
    by_id: &std::collections::BTreeMap<&str, &Issue>,
    id: &str,
    parents: bool,
) -> Vec<(String, String, String)> {
    let ids = if parents {
        book.parents.get(id)
    } else {
        book.children.get(id)
    };
    ids.map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|other| {
            by_id.get(other.as_str()).map_or_else(
                || {
                    (
                        other.clone(),
                        "(missing from the book)".into(),
                        "missing".into(),
                    )
                },
                |issue| (other.clone(), issue.title.clone(), issue.status.clone()),
            )
        })
        .collect()
}

fn edge_line(id: &str, title: &str, status: &str) -> Markup {
    html! {
        li {
            a href=(link("/issue", None, &[("id", id)])) { code { (id) } }
            " " (status_badge(status)) " " (title)
        }
    }
}

fn board_html(columns: &[Column], project: Option<&str>, query: &str, limit: usize) -> Markup {
    html! {
        div class="board" {
            @for column in columns {
                div class="col" {
                    h3 {
                        (column.title)
                        span class="n" { (thousands(column.count() as i64)) }
                    }
                    @if column.cards.is_empty() {
                        p class="empty" { "none" }
                    }
                    @for card in column.cards.iter().take(limit) {
                        (card_html(card, project))
                    }
                    @if column.count() > limit {
                        a class="more" href=(link(
                            "/board",
                            project,
                            &[("q", query), ("status", &column.status)],
                        )) {
                            "… " (thousands((column.count() - limit) as i64)) " more"
                        }
                    }
                }
            }
        }
    }
}

fn card_html(card: &model::Card, project: Option<&str>) -> Markup {
    html! {
        a class="card" href=(link("/issue", project, &[("id", &card.id)])) {
            div class="card-top" {
                span class="cid" { (card.id) }
                span class=(format!("prio p{}", card.priority.clamp(0, 3))) {
                    "p" (card.priority)
                }
            }
            div class="card-title" title=(card.title) { (card.title) }
            div class="card-meta" {
                @if !card.project.is_empty() {
                    span class="proj" title=(card.project) { (card.project) }
                }
                @if let Some(who) = &card.assignee {
                    span class="who" title=(who) { (who) }
                }
                @for label in card.labels.iter().take(2) {
                    span class="tag" title=(label) { (label) }
                }
                @if card.labels.len() > 2 {
                    span class="tag more" { "+" (card.labels.len() - 2) }
                }
            }
        }
    }
}

/// The burndown as an inline SVG. No chart library, no canvas, no script: the
/// browser receives the lines already computed.
///
/// The SVG is the one place markup is not built by `html!`. Every value in it is
/// a number or a date this module formats, never operator text, so it is safe to
/// pass through as [`PreEscaped`].
fn burndown_chart(burn: &Burndown, days: i64) -> Markup {
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
            points.push_str(&format!("{:.1},{:.1} ", x(index), y(get(day))));
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
        grid.push_str(&format!(
            "<line x1=\"{LEFT}\" y1=\"{at:.1}\" x2=\"{:.1}\" y2=\"{at:.1}\" stroke=\"#222e3a\" stroke-width=\"1\"/><text x=\"{:.1}\" y=\"{:.1}\" fill=\"#5d6b7a\" font-size=\"10\" text-anchor=\"end\">{}</text>",
            W - RIGHT,
            LEFT - 6.0,
            at + 3.0,
            thousands(value)
        ));
    }
    let first = burn.days.first().map(|day| day.at).unwrap_or(0);
    let last = burn.days.last().map(|day| day.at).unwrap_or(0);
    let labels = format!(
        "<text x=\"{LEFT}\" y=\"{:.1}\" fill=\"#5d6b7a\" font-size=\"10\" text-anchor=\"start\">{}</text><text x=\"{:.1}\" y=\"{:.1}\" fill=\"#5d6b7a\" font-size=\"10\" text-anchor=\"end\">{}</text>",
        H - 8.0,
        date_label(first),
        W - RIGHT,
        H - 8.0,
        date_label(last)
    );
    let area = {
        let mut points = format!("{:.1},{:.1} ", LEFT, TOP + plot_h);
        for (index, day) in burn.days.iter().enumerate() {
            points.push_str(&format!("{:.1},{:.1} ", x(index), y(day.remaining)));
        }
        points.push_str(&format!("{:.1},{:.1}", W - RIGHT, TOP + plot_h));
        format!("<polygon fill=\"#6ea8fe14\" points=\"{points}\"/>")
    };
    let svg = format!(
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
    );
    html! { (PreEscaped(svg)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_values_are_percent_encoded() {
        assert_eq!(url_encode("personal/avery"), "personal%2Favery");
        assert_eq!(url_encode("a b"), "a+b");
    }

    #[test]
    fn operator_text_is_escaped_by_the_template() {
        let mut issue = Issue {
            id: "marbles-1".into(),
            title: "<script>alert(1)</script>".into(),
            description: "<img src=x onerror=alert(2)>".into(),
            project: "demo".into(),
            status: "open".into(),
            issue_type: "task".into(),
            ..Issue::default()
        };
        issue.labels = vec!["<b>".into()];
        let book = Book {
            issues: vec![issue],
            ..Book::default()
        };
        let page = issue_page(&book, "marbles-1", None, "");
        assert!(
            page.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
            "{page}"
        );
        assert!(
            page.contains("&lt;img src=x onerror=alert(2)&gt;"),
            "{page}"
        );
        assert!(!page.contains("<script>alert(1)</script>"), "{page}");
    }
}
