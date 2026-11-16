-- Atlas: a three-page attention face.
--
-- home  : time, the thing that needs you, the next calendar event, statuses
-- rings : Apple-style move/heart/power rings
-- agenda: the next few calendar items
--
-- This is a design-preview face (`--export`); the pages/rings/icon fields are
-- the next WF1 contract, not the current single-page compiler.

return {
  schema = "watchface.v1",
  name = "atlas",
  pages = {
    {
      id = "home",
      refresh_ms = 1000,
      background = "#000000",
      widgets = {
        { id = "time", kind = "label", text = "{time}", font = "mono36",
          color = "#ffffff", align = "top_left", x = 16, y = 10 },
        { id = "date", kind = "label", text = "{date}", font = "mono20",
          color = "#8a8a8a", align = "top_left", x = 16, y = 52 },
        { id = "battery_icon", kind = "icon", symbol = "battery", font = "fa20",
          color = "#8a8a8a", align = "top_right", x = -52, y = 16 },
        { id = "battery", kind = "label", text = "{battery}%", font = "mono20",
          color = "#8a8a8a", align = "top_right", x = -16, y = 16 },
        { id = "link", kind = "dot", tone = "ble", color = "#2ed3ff",
          align = "top_right", x = -6, y = 48, w = 8, h = 8 },

        { id = "kicker", kind = "label", text = "{alert.kicker}", font = "mono20",
          color = "#2ed3ff", tone = "alert", align = "center", x = 0, y = -40 },
        { id = "message", kind = "label", text = "{alert.title}", font = "mono28",
          color = "#ffffff", align = "center", x = 0, y = 0, w = 220, wrap = true },

        { id = "status_agent", kind = "icon", symbol = "robot", font = "fa20",
          color = "#ffd60a", align = "center", x = -32, y = 50 },
        { id = "status_music", kind = "icon", symbol = "music", font = "fa20",
          color = "#2ed3ff", align = "center", x = 0, y = 50 },
        { id = "status_decision", kind = "icon", symbol = "bell", font = "fa20",
          color = "#ff9f0a", align = "center", x = 32, y = 50 },

        { id = "calendar_icon", kind = "icon", symbol = "calendar", font = "fa20",
          color = "#2ed3ff", align = "bottom_left", x = 16, y = -34 },
        { id = "calendar_time", kind = "label", text = "{calendar.next.time}",
          font = "mono20", color = "#ffffff", align = "bottom_left", x = 44, y = -34 },
        { id = "calendar_title", kind = "label", text = "{calendar.next.title}",
          font = "mono20", color = "#8a8a8a", align = "bottom_left", x = 44, y = -14,
          w = 180 },
      },
    },
    {
      id = "rings",
      refresh_ms = 1000,
      background = "#000000",
      rings = {
        x = 76, y = 108, diameter = 140, gap = 7, thickness = 9,
        items = {
          { id = "move", label = "MOVE", value = "{rings.move}", max = 500, color = "#fa114f" },
          { id = "exercise", label = "EXERCISE", value = "{rings.exercise}", max = 30, color = "#30d158" },
          { id = "stand", label = "STAND", value = "{rings.stand}", max = 12, color = "#0a84ff" },
        },
      },
      widgets = {
        { id = "time", kind = "label", text = "{time}", font = "mono14",
          color = "#8a8a8a", align = "top_left", x = 16, y = 10 },
        { id = "date", kind = "label", text = "{date}", font = "mono14",
          color = "#8a8a8a", align = "top_right", x = -16, y = 10 },

        { id = "move_label", kind = "label", text = "MOVE", font = "mono14",
          color = "#8a8a8a", align = "top_left", x = 156, y = 44 },
        { id = "move_value", kind = "label", text = "{rings.move}", font = "mono20",
          color = "#fa114f", align = "top_left", x = 156, y = 60 },
        { id = "move_unit", kind = "label", text = "KCAL", font = "mono14",
          color = "#8a8a8a", align = "top_left", x = 196, y = 66 },
        { id = "exercise_label", kind = "label", text = "EXERCISE", font = "mono14",
          color = "#8a8a8a", align = "top_left", x = 156, y = 88 },
        { id = "exercise_value", kind = "label", text = "{rings.exercise}", font = "mono20",
          color = "#30d158", align = "top_left", x = 156, y = 104 },
        { id = "exercise_unit", kind = "label", text = "MIN", font = "mono14",
          color = "#8a8a8a", align = "top_left", x = 196, y = 110 },
        { id = "stand_label", kind = "label", text = "STAND", font = "mono14",
          color = "#8a8a8a", align = "top_left", x = 156, y = 132 },
        { id = "stand_value", kind = "label", text = "{rings.stand}", font = "mono20",
          color = "#0a84ff", align = "top_left", x = 156, y = 148 },
        { id = "stand_unit", kind = "label", text = "/12 HR", font = "mono14",
          color = "#8a8a8a", align = "top_left", x = 170, y = 154 },

        { id = "center_fire", kind = "icon", symbol = "fire", font = "fa20",
          color = "#ff9f0a", align = "center", x = -44, y = -12 },

        { id = "status_agent", kind = "icon", symbol = "robot", font = "fa14",
          color = "#ffd60a", align = "top_left", x = 156, y = 176 },
        { id = "status_music", kind = "icon", symbol = "music", font = "fa14",
          color = "#2ed3ff", align = "top_left", x = 178, y = 176 },
        { id = "status_decision", kind = "icon", symbol = "bell", font = "fa14",
          color = "#ff9f0a", align = "top_left", x = 200, y = 176 },

        { id = "calendar_icon", kind = "icon", symbol = "calendar", font = "fa14",
          color = "#2ed3ff", align = "bottom_left", x = 16, y = -14 },
        { id = "calendar_time", kind = "label", text = "{calendar.next.time}",
          font = "mono14", color = "#ffffff", align = "bottom_left", x = 38, y = -14 },
        { id = "calendar_title", kind = "label", text = "{calendar.next.title}",
          font = "mono14", color = "#8a8a8a", align = "bottom_left", x = 84, y = -14,
          w = 152 },
      },
    },
    {
      id = "agenda",
      refresh_ms = 1000,
      background = "#000000",
      widgets = {
        { id = "title", kind = "label", text = "NEXT", font = "mono20",
          color = "#8a8a8a", align = "top_left", x = 16, y = 16 },
        { id = "date", kind = "label", text = "{date}", font = "mono20",
          color = "#8a8a8a", align = "top_right", x = -16, y = 16 },
        { id = "time_1", kind = "label", text = "{calendar.1.time}", font = "mono20",
          color = "#2ed3ff", align = "top_left", x = 16, y = 60 },
        { id = "title_1", kind = "label", text = "{calendar.1.title}", font = "mono20",
          color = "#ffffff", align = "top_left", x = 84, y = 60, w = 156 },
        { id = "sub_1", kind = "label", text = "{calendar.1.sub}", font = "mono20",
          color = "#8a8a8a", align = "top_left", x = 84, y = 82 },

        { id = "time_2", kind = "label", text = "{calendar.2.time}", font = "mono20",
          color = "#2ed3ff", align = "top_left", x = 16, y = 118 },
        { id = "title_2", kind = "label", text = "{calendar.2.title}", font = "mono20",
          color = "#ffffff", align = "top_left", x = 84, y = 118, w = 156 },
        { id = "sub_2", kind = "label", text = "{calendar.2.sub}", font = "mono20",
          color = "#8a8a8a", align = "top_left", x = 84, y = 140 },

        { id = "time_3", kind = "label", text = "{calendar.3.time}", font = "mono20",
          color = "#2ed3ff", align = "top_left", x = 16, y = 176 },
        { id = "title_3", kind = "label", text = "{calendar.3.title}", font = "mono20",
          color = "#ffffff", align = "top_left", x = 84, y = 176, w = 156 },
        { id = "sub_3", kind = "label", text = "{calendar.3.sub}", font = "mono20",
          color = "#8a8a8a", align = "top_left", x = 84, y = 198 },
      },
    },
  },
}
