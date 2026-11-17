//! Where the list and the preview come from.
//!
//! Two builds, one window. The shipped Spore reads one granted workspace through
//! `soil:vfs`: paths and bytes both come from the host, and a path the host does
//! not grant does not exist. The `demo` build exists because a host that has not
//! linked that grant cannot instantiate a component which imports it — so the
//! arrangement can still be looked at, against a fixed listing.

use crate::Row;

/// Read a directory's entries, sorted with directories first.
#[cfg(not(feature = "demo"))]
pub fn ls(path: &str) -> Result<Vec<Row>, String> {
    use crate::soil::vfs::vfs;
    match vfs::ls(path, 0, crate::PAGE) {
        Ok(page) => Ok(sort(
            page.entries
                .into_iter()
                .map(|entry| Row {
                    name: if entry.name.is_empty() {
                        entry.path.clone()
                    } else {
                        entry.name.clone()
                    },
                    path: entry.path,
                    directory: matches!(entry.kind, vfs::Kind::Directory),
                    size: entry.size,
                    hydrated: matches!(entry.hydration, vfs::Hydration::Present),
                    media_type: entry.media_type,
                    updated_ms: entry.updated_at_ms,
                })
                .collect(),
        )),
        Err(error) => Err(format!("{error:?}")),
    }
}

/// Read the first `max` bytes of a file: bytes, content id, and whether that was
/// the whole object.
#[cfg(not(feature = "demo"))]
pub fn read(path: &str, max: u32) -> Result<(Vec<u8>, String, bool), String> {
    use crate::soil::vfs::vfs;
    match vfs::read(path, 0, max) {
        Ok(chunk) => Ok((chunk.data, chunk.content_id, chunk.eof)),
        Err(error) => Err(format!("{error:?}")),
    }
}

/// What a repository says about the directory the window is in.
///
/// `None` covers both "the host refused `soil:git`" and "this is not a
/// repository". The window shows nothing in either case: a browser that guessed
/// between them would be describing a tree it cannot see.
pub struct Repo {
    pub branch: String,
    pub head: String,
    pub dirty: bool,
    pub changed: u32,
    pub ahead: u32,
    pub behind: u32,
    pub has_upstream: bool,
}

#[cfg(not(feature = "demo"))]
pub fn git(path: &str) -> Option<Repo> {
    use crate::soil::git::git;
    let status = git::status(path).ok()?;
    Some(Repo {
        branch: status.branch,
        head: status.head,
        dirty: status.dirty,
        changed: status.changed,
        ahead: status.ahead,
        behind: status.behind,
        has_upstream: status.has_upstream,
    })
}

/// A fixed repository for the harness, so the captured frame shows the chip.
#[cfg(feature = "demo")]
pub fn git(path: &str) -> Option<Repo> {
    path.ends_with("projects").then(|| Repo {
        branch: "main".to_owned(),
        head: "9c1de2f".to_owned(),
        dirty: true,
        changed: 2,
        ahead: 1,
        behind: 0,
        has_upstream: true,
    })
}

/// One Desktop the machine offers, as the picker shows it.
#[derive(Clone)]
pub struct DesktopRow {
    pub id: String,
    pub name: String,
    /// `bound`, `available` or `denied`: the picker shows refused Desktops
    /// rather than hiding them, because "you cannot open this" and "this does
    /// not exist" are different facts.
    pub access: String,
}

#[cfg(not(feature = "demo"))]
pub fn desktops() -> Vec<DesktopRow> {
    use crate::soil::desktops::desktops;
    match desktops::available() {
        Ok(rows) => rows
            .into_iter()
            .map(|row| DesktopRow {
                id: row.id,
                name: row.name,
                access: match row.access {
                    desktops::Access::Bound => "bound",
                    desktops::Access::Available => "available",
                    desktops::Access::Denied => "denied",
                }
                .to_owned(),
            })
            .collect(),
        // A host without the grant offers no picker at all; an empty list is the
        // honest answer, and the window shows no chip.
        Err(_) => Vec::new(),
    }
}

/// Bind this window to another Desktop. `Ok` means the host has already swapped
/// the workspace, so the caller's next listing reads the new one.
#[cfg(not(feature = "demo"))]
pub fn open(id: &str) -> Result<String, String> {
    use crate::soil::desktops::desktops;
    desktops::open(id)
        .map(|row| row.name)
        .map_err(|error| format!("{error:?}"))
}

#[cfg(feature = "demo")]
pub fn desktops() -> Vec<DesktopRow> {
    vec![
        DesktopRow {
            id: "fpl".into(),
            name: "FPL".into(),
            access: "bound".into(),
        },
        DesktopRow {
            id: "personal".into(),
            name: "Personal".into(),
            access: "available".into(),
        },
    ]
}

#[cfg(feature = "demo")]
pub fn open(id: &str) -> Result<String, String> {
    Ok(id.to_owned())
}

/// Create a directory. Fails when it already exists: a create that silently
/// succeeds on an existing name is how a folder gets replaced by an empty one.
#[cfg(not(feature = "demo"))]
pub fn mkdir(path: &str) -> Result<(), String> {
    use crate::soil::vfs_mut::vfs_mut;
    vfs_mut::mkdir(path).map_err(|error| format!("{error:?}"))
}

/// Create an empty file.
#[cfg(not(feature = "demo"))]
pub fn new_file(path: &str) -> Result<(), String> {
    use crate::soil::vfs_mut::vfs_mut;
    vfs_mut::write(path, &[], "text/plain", None, None)
        .map(|_| ())
        .map_err(|error| format!("{error:?}"))
}

#[cfg(feature = "demo")]
pub fn mkdir(_path: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(feature = "demo")]
pub fn new_file(_path: &str) -> Result<(), String> {
    Ok(())
}

fn sort(mut rows: Vec<Row>) -> Vec<Row> {
    rows.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| a.name.cmp(&b.name))
    });
    rows
}

/// A fixed listing, so the window can be rendered without a namespace.
#[cfg(feature = "demo")]
pub fn ls(path: &str) -> Result<Vec<Row>, String> {
    let row = |name: &str, directory: bool, size: u64| Row {
        name: name.to_owned(),
        path: if path.is_empty() {
            name.to_owned()
        } else {
            format!("{path}/{name}")
        },
        directory,
        size,
        hydrated: true,
        media_type: String::new(),
        updated_ms: 0,
    };
    let rows = match path {
        "" => vec![
            row("notes", true, 0),
            row("projects", true, 0),
            row("vendor", true, 0),
            row("README.md", false, 1_842),
            row("tend.lua", false, 6),
            row("wit-provenance.txt", false, 914),
            row("design-notes.md", false, 2_310),
            row("render.rs", false, 4_190),
            row("Cargo.toml", false, 402),
            row("budget.xlsx", false, 22_110),
            row("launch.pptx", false, 1_204_300),
            row("thesis.pdf", false, 3_908_772),
            row("sketch.png", false, 88_400),
            row("voice-memo.m4a", false, 512_000),
        ],
        "projects" => vec![
            row("fungos-vfs.md", false, 436),
            row("q3-review.pptx", false, 18),
            row("scope.docx", false, 762),
        ],
        _ => vec![row("today.md", false, 512), row("scratch.md", false, 66)],
    };
    Ok(sort(rows))
}

#[cfg(feature = "demo")]
pub fn read(_path: &str, _max: u32) -> Result<(Vec<u8>, String, bool), String> {
    Ok((
        b"# Scope\n\nA document layer that a Spore can read: blocks with identities,\n\
          a file as a view of them, and threads anchored to blocks rather than to\n\
          byte offsets.\n\nThis pane is what a preview looks like when the bytes are\n\
          already on this machine.\n"
            .to_vec(),
        "blake3:demo".to_owned(),
        true,
    ))
}
