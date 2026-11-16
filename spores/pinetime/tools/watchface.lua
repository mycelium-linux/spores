-- Watch-face authoring compiler.
--
-- A face is a Lua table (see faces/glance.lua). This tool validates it, fills
-- defaults, and emits either JSON for review or the compact WF1 descriptor a
-- native face renderer can parse without a JSON or Lua runtime on the MCU.

local M = {}

local SCHEMA = "watchface.v1"
local MAX_WIDGETS = 16
local MAX_TEXT = 100

local KINDS = {label = 1, dot = 2, icon = 3, bar = 4, ring = 5}
local FONTS = {mono14 = 1, mono20 = 2, mono28 = 3, mono36 = 4, fa14 = 5, fa20 = 6, fa28 = 7}
local ALIGNS = {top_left = 1, top_right = 2, center = 3, bottom_left = 4, bottom_right = 5}
-- A tone is resolved by the renderer from local state; it overrides color.
local TONES = {none = 0, alert = 1, ble = 2}

local function fail(message)
  error(message, 0)
end

local function check(condition, message)
  if not condition then
    fail(message)
  end
end

local function is_int(value)
  return type(value) == "number" and value % 1 == 0
end

local function is_color(value)
  return type(value) == "string" and value:match("^#%x%x%x%x%x%x$") ~= nil
end

local function rgb565(hex)
  local r = tonumber(hex:sub(2, 3), 16)
  local g = tonumber(hex:sub(4, 5), 16)
  local b = tonumber(hex:sub(6, 7), 16)
  return ((r & 0xf8) << 8) | ((g & 0xfc) << 3) | (b >> 3)
end

local function normalize_widget(widget, index)
  check(type(widget) == "table", "widget " .. index .. " must be a table")
  check(type(widget.id) == "string" and widget.id:match("^[a-z0-9_]+$"),
    "widget " .. index .. " needs a lowercase id")
  check(KINDS[widget.kind] ~= nil, "widget " .. widget.id .. " has unknown kind " .. tostring(widget.kind))

  local text = widget.text or ""
  check(type(text) == "string", "widget " .. widget.id .. " text must be a string")
  check(#text <= MAX_TEXT, "widget " .. widget.id .. " text exceeds the watch budget")
  if widget.kind == "label" then
    check(#text > 0, "widget " .. widget.id .. " label needs text")
  end

  local font = widget.font or "mono20"
  check(FONTS[font] ~= nil, "widget " .. widget.id .. " has unknown font " .. tostring(font))

  local align = widget.align or "center"
  check(ALIGNS[align] ~= nil, "widget " .. widget.id .. " has unknown align " .. tostring(align))

  local color = widget.color or "#ffffff"
  check(is_color(color), "widget " .. widget.id .. " needs #rrggbb color")

  local tone = widget.tone or "none"
  check(TONES[tone] ~= nil, "widget " .. widget.id .. " has unknown tone " .. tostring(tone))

  local x = widget.x or 0
  local y = widget.y or 0
  check(is_int(x) and x >= -239 and x <= 239, "widget " .. widget.id .. " x out of range")
  check(is_int(y) and y >= -239 and y <= 239, "widget " .. widget.id .. " y out of range")

  local width = widget.w or 0
  local height = widget.h or 0
  check(is_int(width) and width >= 0 and width <= 240, "widget " .. widget.id .. " w out of range")
  check(is_int(height) and height >= 0 and height <= 240, "widget " .. widget.id .. " h out of range")
  if widget.kind == "dot" then
    check(width > 0 and height > 0, "widget " .. widget.id .. " dot needs w/h")
  end

  return {
    id = widget.id,
    kind = widget.kind,
    text = text,
    font = font,
    align = align,
    color = color,
    tone = tone,
    x = x,
    y = y,
    w = width,
    h = height,
    wrap = widget.wrap == true,
  }
end

function M.validate(face)
  check(type(face) == "table", "face must return a table")
  check(face.schema == SCHEMA, "face schema must be " .. SCHEMA)
  check(type(face.name) == "string" and face.name:match("^[a-z0-9_%-]+$"),
    "face needs a lowercase name")

  local refresh = face.refresh_ms or 1000
  check(is_int(refresh) and refresh >= 100 and refresh <= 60000, "refresh_ms out of range")

  local background = face.background or "#000000"
  check(is_color(background), "background needs #rrggbb")

  check(type(face.widgets) == "table" and #face.widgets >= 1 and #face.widgets <= MAX_WIDGETS,
    "face needs 1.." .. MAX_WIDGETS .. " widgets")

  local widgets = {}
  local seen = {}
  for index, widget in ipairs(face.widgets) do
    local normalized = normalize_widget(widget, index)
    check(not seen[normalized.id], "duplicate widget id " .. normalized.id)
    seen[normalized.id] = true
    widgets[#widgets + 1] = normalized
  end

  return {
    schema = SCHEMA,
    name = face.name,
    refresh_ms = refresh,
    background = background,
    widgets = widgets,
  }
end

local function escape(text)
  return '"' .. text:gsub('[%z\1-\31\\"]', function(char)
    if char == '"' then
      return '\\"'
    elseif char == '\\' then
      return '\\\\'
    elseif char == '\n' then
      return '\\n'
    elseif char == '\r' then
      return '\\r'
    elseif char == '\t' then
      return '\\t'
    end
    return string.format('\\u%04x', char:byte())
  end) .. '"'
end

local function encode(value, level)
  level = level or 0
  local kind = type(value)
  if kind == "nil" then
    return "null"
  elseif kind == "boolean" then
    return value and "true" or "false"
  elseif kind == "number" then
    check(value % 1 == 0, "JSON only supports integers here")
    return string.format("%d", value)
  elseif kind == "string" then
    return escape(value)
  elseif kind == "table" then
    local pad = string.rep("  ", level)
    local inner = string.rep("  ", level + 1)
    if #value > 0 then
      local parts = {}
      for _, item in ipairs(value) do
        parts[#parts + 1] = inner .. encode(item, level + 1)
      end
      return "[\n" .. table.concat(parts, ",\n") .. "\n" .. pad .. "]"
    end
    local keys = {}
    for key in pairs(value) do
      check(type(key) == "string", "JSON object keys must be strings")
      keys[#keys + 1] = key
    end
    table.sort(keys)
    local parts = {}
    for _, key in ipairs(keys) do
      parts[#parts + 1] = inner .. escape(key) .. ": " .. encode(value[key], level + 1)
    end
    return "{\n" .. table.concat(parts, ",\n") .. "\n" .. pad .. "}"
  end
  fail("cannot encode " .. kind)
end

function M.json(face)
  return encode(M.validate(face))
end

-- Design-preview export: no WF1 validation, so pages/rings/icons can be
-- iterated before the firmware contract catches up. Never shipped as a face.
function M.export(face)
  check(type(face) == "table", "face must return a table")
  return encode(face)
end

function M.compile(face)
  local normalized = M.validate(face)
  local out = {
    "WF1\0",
    string.pack("<BBHH", 1, #normalized.widgets, normalized.refresh_ms, rgb565(normalized.background)),
  }
  for _, widget in ipairs(normalized.widgets) do
    out[#out + 1] = string.pack(
      "<BBBBBHhhHH",
      KINDS[widget.kind],
      widget.wrap and 1 or 0,
      TONES[widget.tone],
      FONTS[widget.font],
      ALIGNS[widget.align],
      rgb565(widget.color),
      widget.x,
      widget.y,
      widget.w,
      widget.h,
      #widget.text)
    out[#out + 1] = widget.text
  end
  return table.concat(out)
end

function M.main(args)
  local path = args[1]
  check(path ~= nil, "usage: lua tools/watchface.lua <face.lua> [--json | --bin OUT]")
  local chunk, load_error = loadfile(path)
  check(chunk ~= nil, load_error)
  local face = chunk()
  local mode = args[2] or "--json"
  if mode == "--json" then
    print(M.json(face))
  elseif mode == "--export" then
    print(M.export(face))
  elseif mode == "--bin" then
    local output = args[3]
    check(output ~= nil, "--bin needs an output path")
    local bytes = M.compile(face)
    local file = assert(io.open(output, "wb"))
    file:write(bytes)
    file:close()
    print(string.format("%s: %d bytes", output, #bytes))
  else
    fail("unknown mode " .. mode)
  end
end

if arg and arg[0] and arg[0]:match("watchface%.lua$") then
  M.main(arg)
end

return M
