-- The Glance face, as data instead of another C++ WatchFace class.
--
-- Run through tools/watchface.lua to validate and compile it. The firmware
-- renderer owns fonts, colors, and LVGL; this file owns only what appears and
-- which value feeds it. `{time}`, `{battery}`, `{hr}`, `{steps}`, and
-- `{alert.*}` are resolved on the watch; a future FaceState channel can add
-- `{music.*}`, `{decision.*}`, and `{agent.*}` without a firmware change.

return {
  schema = "watchface.v1",
  name = "glance",
  refresh_ms = 1000,
  background = "#000000",
  widgets = {
    {
      id = "time",
      kind = "label",
      text = "{time}",
      font = "mono20",
      color = "#8a8a8a",
      align = "top_left",
      x = 16,
      y = 16,
    },
    {
      id = "battery",
      kind = "label",
      text = "{battery}%",
      font = "mono20",
      color = "#8a8a8a",
      align = "top_right",
      x = -20,
      y = 16,
    },
    {
      id = "link",
      kind = "dot",
      color = "#2ed3ff",
      tone = "ble",
      align = "top_right",
      x = -6,
      y = 21,
      w = 8,
      h = 8,
    },
    {
      id = "kicker",
      kind = "label",
      text = "{alert.kicker}",
      font = "mono20",
      color = "#2ed3ff",
      tone = "alert",
      align = "center",
      x = 0,
      y = -44,
    },
    {
      id = "message",
      kind = "label",
      text = "{alert.title}",
      font = "mono20",
      color = "#ffffff",
      align = "center",
      x = 0,
      y = -6,
      w = 200,
      wrap = true,
    },
    {
      id = "steps",
      kind = "label",
      text = "STEPS {steps}",
      font = "mono20",
      color = "#8a8a8a",
      align = "bottom_left",
      x = 16,
      y = -18,
    },
    {
      id = "heart",
      kind = "label",
      text = "HR {hr}",
      font = "mono20",
      color = "#8a8a8a",
      align = "bottom_right",
      x = -16,
      y = -18,
    },
  },
}
