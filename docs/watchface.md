# Watch faces: author in Lua, render natively

A watch face is a **view**, not a Spore. The `pinetime` Spore owns the device,
the data, and the BLE link; the watch owns the pixels. Lua is the authoring
language on the host, not a runtime on the nRF52832.

## Why not Lua on the watch

The PineTime has 64 KB RAM and 512 KB flash. LVGL, NimBLE, FreeRTOS, the app,
and the display buffer already share that budget. A Lua VM, its GC, and sensor
bindings would be a battery and reliability tax for a face that redraws once a
second. The host — Mac now, S22 later — has the RAM and already runs Lua and
Canvas.

## The model

A face is a Lua table returned by a file:

```lua
return {
  schema = "watchface.v1",
  name = "glance",
  refresh_ms = 1000,
  background = "#000000",
  widgets = {
    { id = "time", kind = "label", text = "{time}", font = "mono20",
      color = "#8a8a8a", align = "top_left", x = 16, y = 16 },
    -- ...
  },
}
```

`tools/watchface.lua` validates it, fills defaults, and emits either JSON for
review or the compact WF1 descriptor. The firmware parses WF1 and creates LVGL
objects. No Lua bytecode reaches the watch.

See [`faces/glance.lua`](../spores/pinetime/faces/glance.lua) for the full face.

## Data bindings

A label's `text` may contain placeholders:

| Binding | Source |
|---|---|
| `{time}` | local RTC |
| `{battery}` | local battery controller |
| `{hr}` | local heart-rate controller |
| `{steps}` | local motion controller |
| `{alert.kicker}` | local notification manager (`NEW`, `LAST`, `QUIET`) |
| `{alert.title}` | local notification manager |

`tone = "alert"` makes the kicker muted when quiet and accent when a decision is
live; `tone = "ble"` makes the link dot accent when connected and dim when not.
The color field is the fallback for a renderer that has no state yet.
| `{music.track}`, `{music.artist}` | FaceState from the host |
| `{decision.summary}`, `{agent.prompt}` | FaceState from the host |
| `{vitals.hr}`, `{vitals.steps}` | FaceState from the host |
| `{rings.move}`, `{rings.exercise}`, `{rings.stand}` | Apple-style ring values from the host (active kcal, exercise minutes, stand hours) |

Local values update without BLE. Host values arrive on a small FaceState
characteristic; the face never waits on the radio for time, battery, HR, or
steps.

## Schema

Top level:

| Field | Type | Default | Notes |
|---|---|---|---|
| `schema` | string | required | must be `watchface.v1` |
| `name` | string | required | lowercase `a-z0-9_-` |
| `refresh_ms` | integer | `1000` | 100..60000 |
| `background` | `#rrggbb` | `#000000` | RGB565 on device |
| `widgets` | array | required | 1..16 widgets |

Widget:

| Field | Type | Default | Notes |
|---|---|---|---|
| `id` | string | required | unique, `a-z0-9_` |
| `kind` | enum | required | `label`, `dot`, `icon`, `bar`, `ring` |
| `text` | string | `""` | labels only; <= 100 bytes |
| `font` | enum | `mono20` | `mono14`, `mono20`, `mono28`, `mono36`, `fa14`, `fa20`, `fa28` |
| `color` | `#rrggbb` | `#ffffff` | RGB565 on device; fallback when `tone` is `none` |
| `tone` | enum | `none` | `none`, `alert`, `ble`; renderer resolves `alert` to muted/accent and `ble` to accent/dim |
| `align` | enum | `center` | `top_left`, `top_right`, `center`, `bottom_left`, `bottom_right` |
| `x`, `y` | integer | `0` | -239..239; negative means from the aligned edge |
| `w`, `h` | integer | `0` | 0 means auto for labels; dots need both |
| `wrap` | boolean | `false` | label text wraps inside `w` |

`dot` is a filled circle; `icon`, `bar`, and `ring` are reserved for the
renderer and compile today but are not drawn yet.

## Limits

- 1..16 widgets.
- 100 bytes of text per label, the same ANS budget.
- Coordinates fit the 240x240 display.
- No arbitrary code, file access, or network from a face.
- A typical face is well under one BLE write: Glance is 178 bytes.

## WF1 binary layout

Little-endian:

```text
magic        4 bytes  "WF1\0"
version      u8       1
widget_count u8       1..16
refresh_ms   u16
background   u16      RGB565

per widget:
kind         u8       1 label, 2 dot, 3 icon, 4 bar, 5 ring
flags        u8       bit 0 wrap
tone         u8       0 none, 1 alert, 2 ble
font         u8       1 mono14, 2 mono20, 3 mono28, 4 mono36,
                      5 fa14, 6 fa20, 7 fa28
align        u8       1 top_left, 2 top_right, 3 center,
                      4 bottom_left, 5 bottom_right
color        u16      RGB565
x            i16
y            i16
w            u16
h            u16
text_len     u16
 text        text_len UTF-8 bytes
```

## Compile

```sh
cd spores/pinetime
lua tools/watchface.lua faces/glance.lua --json
lua tools/watchface.lua faces/glance.lua --bin /tmp/glance.wf
lua tools/watchface_test.lua
```

My `lua` is 5.4+; `string.pack` is required.

## Design preview: pages, rings, and statuses

[`faces/atlas.lua`](../spores/pinetime/faces/atlas.lua) is the next design: a
three-page attention face with a home card, Apple-style rings, and a calendar
agenda. It uses fields that are not in WF1 v1 yet:

- `pages = { ... }` with `id`, `refresh_ms`, and `background` per page
- `rings = { x, y, diameter, gap, thickness, items }`
- `{rings.move}`, `{rings.exercise}`, `{rings.stand}` bindings for the three
  Apple-style rings (active kcal, exercise minutes, stand hours)
- `kind = "icon"` with `symbol = "calendar"`, `robot`, `music`, `bell`, `fire`, etc.
- `visible_if = "agent.needs_you"`, `music.playing`, `decision.pending`
- calendar bindings `{calendar.next.*}` and `{calendar.1..3.*}`

Use `--export` for a design preview until the renderer and WF1 v2 land:

```sh
lua tools/watchface.lua faces/atlas.lua --export
```

`--export` deliberately skips WF1 validation; it is for review, not a package.

## Firmware contract

The renderer is the next firmware step:

1. Parse WF1 into a fixed widget array (no heap per widget).
2. Create LVGL labels/dots once at face load.
3. Resolve local bindings on the refresh tick; host bindings from `FaceState`.
4. Keep time on the local RTC; never stream a framebuffer.
5. Store the active descriptor and FaceState in a small GATT service so the host
   can select/reload a face without reflashing.

The existing `WatchFaceGlance` is the reference implementation; the descriptor
above reproduces its layout.

## Not in this contract

- Lua bytecode or a VM on the MCU.
- Full framebuffer or display-list streaming: 240x240 RGB565 is about 115 KB per
  frame, too much for animation and battery.
- Arbitrary drawing. If a face needs custom drawing or animation, keep it as a
  native C++ face.
