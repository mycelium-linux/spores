# The board

The Marbles work graph on one surface. The board reads the `marbles`
collection through `soil:records` and draws three things from it:

- a **kanban** of every issue grouped by status, in the tracker's own order
  (`open`, `in_progress`, `review`, `blocked`, `deferred`, `closed`);
- a **burndown** over the last fourteen days, drawing what is left against the
  pace that would have cleared the same scope evenly;
- a **search** over the short id, the title, the project, the labels and who
  holds it, narrowing both views as you type.

# Statuses it does not know

A status this build has never heard of still gets a column, appended after the
six. An issue that exists but is not drawn is exactly the failure a board is
meant to prevent, so the board widens rather than drops.

# Keys

- **printable text** narrows the search.
- **backspace** deletes a character.
- **tab** switches between the board and the burndown.
- **escape** clears the search; with nothing to clear, quits.
- **command-R** re-reads the store.

# Where "now" comes from

A Spore has no clock grant. The newest record in the book is the latest moment
anything happened, and that is what the burndown window ends at — so a capture
of a store reproduces the same chart whenever it is taken.

# What it cannot do

The board is read-only by grant: it holds `soil:records` and never writes, so
the tracker stays the single writer of its own graph. It opens no socket and
reads no file — the store is the host's to open.

# Launching it

```sh
soil spore run spores/board --collection marbles --book marbles
```

`--collection` names the collection the guest asks for *and* is the grant that
opens it. Marbles keeps every record type in one collection named `marbles`
with `issue/[...]` entities inside it, so the name is given on the command line
rather than declared in the manifest, whose `[records]` table expects schema
ids (`area/name@version`).

# Rendering

GPUI lays the view out in the guest and renders it offscreen through
`wasi:webgpu`; `soil:present` hands the texture to whatever is presenting. No
window, swapchain, or native handle crosses that boundary, so the same module
draws the same board on every host.
