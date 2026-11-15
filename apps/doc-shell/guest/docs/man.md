# The reader

`doc-shell` opens one Word file as blocks rather than as a page image. The
manuscript is parsed from its own bytes — the guest has no filesystem, so what it
shows came through the one granted workspace — and every paragraph is labelled
with the `w14:paraId` Word already wrote into it. Two readers of the same file
agree on block identity without a store between them.

# The vault

The left pane lists the workspace the host granted, one level at a time. A file
larger than the host's read budget is paged, not truncated. Nothing outside that
workspace exists: a path it cannot see is a document it cannot open, and the pane
says so instead of inventing a listing.

# Comments

A comment is a pending entity, not an edit to the text. It lives in the Spore's
records book, syncs like any other record, can be replied to and resolved, and
carries the principal the host attested rather than a name somebody typed. When
the host refuses the write the shell says `could not save the comment` and keeps
the text; a comment that silently vanished would be worse than a visible refusal.

# Keys

`c` opens the composer on the selected block, `x` resolves a thread, `?` lists
every key, and `q` quits. The same keys work under a window and under a capture,
because the Spore reads `soil:present/input` and nothing else.

# What it cannot do

It cannot save the document: writing back is `soil:vfs-mut`, a separate grant
with a separate review, and this Spore deliberately does not hold it. It has no
network, no process, no clock and no raw filesystem. Accepting a comment into the
text would be a mutation of the file, so it is a future Spore, not a hidden mode
of this one.
