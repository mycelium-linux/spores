# deadwax

## Name

deadwax — controller for the household playhead.

## Synopsis

```text
deadwax
```

## Description

One window over the house's music. The left pane searches the `deadwax`
catalog; the right pane is what the playback owner is doing: the current
track, its playhead, the queue, and who asked for what.

A local track is named by its **CIDv1** — the address of the canonical master —
and a Spotify track by its URI. The owner resolves either. This Spore never
reads a library file, holds a Spotify token, or speaks to another node: those
belong to the host, which bridges Unibus's `audio.playback.*` contract to the
`soil:playback` calls below.

## Keys

| Key | Effect |
|---|---|
| `/` | Search the current source |
| `Enter` | Play the selected track now |
| `a` | Queue the selected track after the current one |
| `Space` | Play/pause |
| `n` / `p` | Next / previous |
| `j` / `k`, arrows | Move in the list |
| `s` | Switch source (library / Spotify) |
| `r` | Refresh the list |
| `q` | Close the window |

## Grants

- `soil:playback` — read the playhead, send transport commands, search a source
  through the owner.
- `soil:http` — one bounded `GET` against the local catalog at
  `127.0.0.1:6601`.
- `soil:present` — draw its own surface through the host's WebGPU device.

## Environment

The catalog origin is a constant in the module, not an argument: a Spore names
its origin rather than accepting one. Run `deadwax serve --listen
127.0.0.1:6601` on the host.

## See also

- `soil:playback` in `wit/deps/soil-playback/package.wit`
- `unibus`'s `docs/playback-contract.md` — the wire contract this mirrors
- `spores/terminal-gpui` — the reference GPUI Spore
