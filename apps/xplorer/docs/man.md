# The grid

`xplorer` shows one granted workspace as a grid of tiles: an icon above a name,
wrapping to as many columns as fit, with the selected entry previewed beside it.
It is a view over what the host says exists, not a walk of a filesystem — the
listing, the sizes and the content ids all come from `soil:vfs`.

# Previews

A preview is a bounded read plus a renderer per media type. Heavy decoders are
loaded only when something of that type is selected, and a file larger than the
budget reports `over-budget` rather than showing a truncated thing that looks
complete.

# Keys

Arrow keys move the selection, `d` toggles the theme, `?` lists the keys, and `q`
quits. A directory is marked with a trailing slash because the font has no
triangle to spend on it.

# What it cannot do

It cannot rename, move or delete: those are `soil:vfs-mut`, deliberately not
granted, and a browser that can rename cannot be trusted to describe what it
browsed. There is no network, no process and no clock.
