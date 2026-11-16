-- Host-side check for the watch-face authoring format.
-- Run from spores/pinetime: lua tools/watchface_test.lua

local watchface = dofile("tools/watchface.lua")
local face = dofile("faces/glance.lua")

local normalized = watchface.validate(face)
assert(normalized.name == "glance")
assert(#normalized.widgets == 7)
assert(normalized.widgets[1].id == "time")
assert(normalized.widgets[1].text == "{time}")

local json = watchface.json(face)
assert(json:find('"glance"', 1, true) ~= nil)
assert(json:find('"mono20"', 1, true) ~= nil)

local bytes = watchface.compile(face)
assert(bytes:sub(1, 4) == "WF1\0")
assert(#bytes > 10)
assert(#bytes < 1024, "a face should fit comfortably in one BLE write")

local duplicate = {
  schema = "watchface.v1",
  name = "bad",
  widgets = {
    { id = "x", kind = "label", text = "one" },
    { id = "x", kind = "label", text = "two" },
  },
}
local ok = pcall(function()
  watchface.validate(duplicate)
end)
assert(not ok, "duplicate ids must fail")

local oversized = {
  schema = "watchface.v1",
  name = "bad",
  widgets = {
    { id = "x", kind = "label", text = string.rep("x", 101) },
  },
}
assert(not pcall(function()
  watchface.validate(oversized)
end), "oversized text must fail")

local atlas = dofile("faces/atlas.lua")
local preview = watchface.export(atlas)
assert(preview:find('"pages"', 1, true) ~= nil)
assert(preview:find('"rings"', 1, true) ~= nil)
assert(preview:find("calendar.next.title", 1, true) ~= nil)

print(string.format("watchface: ok, %d widgets, %d bytes; atlas %d pages",
  #normalized.widgets, #bytes, #atlas.pages))
