-- smoke-watchface.lua — load watchfaces/bluetooth-audio/main.lua under a
-- stubbed lvgl binding and exercise the installer flow, catching nil-global /
-- scope bugs and protocol errors.
--
-- Usage: lua scripts/smoke-watchface.lua [path-to-main.lua]
-- Returns non-zero on any load/flow failure.

local TOKEN = "bluetooth_audio"
local DEFAULT = "watchfaces/bluetooth-audio/main.lua"

local lvgl = {}
lvgl.HOR_RES = function() return 336 end
lvgl.VER_RES = function() return 480 end
lvgl.OPA = function(v) return v end
lvgl.FLAG = { SCROLLABLE = 1, CLICKABLE = 2 }
lvgl.ALIGN = { CENTER = 1, TOP_MID = 2, BOTTOM_MID = 3 }

local created = {}
local obj_mt = {}
function obj_mt:clear_flag() return self end
function obj_mt:add_flag() return self end
function obj_mt:set(props) self._last_set = props; return self end
function obj_mt:onClicked(fn) self._click = fn; table.insert(created, self); return self end
function lvgl.Object(parent, props)
    local o = setmetatable({ _parent = parent, _props = props }, { __index = obj_mt })
    table.insert(created, o)
    return o
end
function lvgl.Label(parent, props)
    local o = setmetatable({ _parent = parent, _props = props }, { __index = obj_mt })
    table.insert(created, o)
    return o
end

_G.SCRIPT_PATH = "/fake/"
package.loaded["lvgl"] = lvgl

local original_io_open = io.open
local original_os_execute = os.execute

local function icon_fixture()
    return string.char(0x19, 0x10, 0, 0, 2, 0, 2, 0, 8, 0, 0, 0)
        .. string.rep("\0", 16)
end

local function installer_io(fault)
    local files = {
        ["/fake/receipt.bin"] = "CMI1" .. string.rep("\0", 252),
        ["/fake/module.bin"] = "\127ELF" .. string.rep("\0", 508),
        ["/fake/long_test_audio_stream.bin"] = "\255\243",
        ["/fake/appicon_headphones.bin"] = icon_fixture(),
    }
    if fault == "missing_icon" then
        files["/fake/appicon_headphones.bin"] = nil
    end
    local device_request
    local function open(path, mode)
        if path == "/dev/canopus" then
            if mode == "wb" then
                return {
                    write = function(_, data)
                        device_request = data
                        if fault == "short_write" then return #data - 1 end
                        return #data
                    end,
                    close = function() return true end,
                }
            end
            local function le32(value)
                return string.char(value % 256, math.floor(value / 256) % 256,
                    math.floor(value / 65536) % 256,
                    math.floor(value / 16777216) % 256)
            end
            local request_id = fault == "stale_response" and 9 or 1
            local response = "2CPC" .. string.char(36, 0, 2, 0, 1, 0, 0, 0)
                .. le32(36) .. le32(2) .. le32(request_id) .. le32(0)
                .. le32(5) .. le32(0)
            return {
                read = function() return response end,
                close = function() return true end,
            }
        end
        if mode == "rb" then
            if files[path] == nil then return nil end
            return {
                read = function() return files[path] end,
                close = function() return true end,
            }
        end
        if mode == "wb" then
            return {
                write = function(_, data) files[path] = data; return true end,
                close = function() return true end,
            }
        end
        return nil
    end
    return open, function()
        return device_request, files
    end
end

local function check(path, fault)
    created = {}
    local state
    io.open, state = installer_io(fault)
    os.execute = function(command)
        local _, files = state()
        if command == "cp /fake/long_test_audio_stream.bin /data/canopus/tmp_btaudio_module_long_audio_test.mp3" then
            files["/data/canopus/tmp_btaudio_module_long_audio_test.mp3"] =
                files["/fake/long_test_audio_stream.bin"]
        end
        return true
    end
    local ok, err = pcall(dofile, path)
    io.open = original_io_open
    os.execute = original_os_execute
    if not ok then
        print("LOAD FAIL:", path, err)
        return false
    end
    local request, files = state()
    if fault == "missing_icon" then
        if request ~= nil then
            print("ICON FAIL-CLOSED REQUEST FAIL:", path)
            return false
        end
        local diagnosed = false
        for _, object in ipairs(created) do
            local text = object._last_set and object._last_set.text
            if type(text) == "string" and text:match("Install failed") then
                diagnosed = true
            end
        end
        if not diagnosed then
            print("ICON DIAGNOSTIC FAIL:", path)
            return false
        end
        print("watchface icon staging failure handled: " .. path)
        return true
    end
    local suffix = TOKEN .. "\0"
    if type(request) ~= "string" or request:sub(1, 4) ~= "2CPC"
        or request:byte(17) ~= 2
        or request:sub(-#suffix) ~= suffix
        or files["/data/canopus/inbox/" .. TOKEN .. ".cmi"] == nil
        or files["/data/canopus/inbox/" .. TOKEN .. ".ko"] == nil
        or files["/data/canopus/appicon_headphones.bin"] ~= icon_fixture() then
        print("INSTALL FLOW FAIL:", path)
        return false
    end
    if fault then
        local diagnosed = false
        for _, object in ipairs(created) do
            local text = object._last_set and object._last_set.text
            if type(text) == "string" and text:match("Install failed") then
                diagnosed = true
            end
        end
        if not diagnosed then
            print("INSTALL DIAGNOSTIC FAIL:", path, fault)
            return false
        end
    end
    local clicks = 0
    for _, o in ipairs(created) do
        if o._click then
            local ok2, err2 = pcall(o._click)
            if not ok2 then
                print("CLICK FAIL:", path, tostring(err2))
                return false
            end
            clicks = clicks + 1
        end
    end
    print(string.format("watchface OK: %s (%d buttons clicked)", path, clicks))
    return true
end

local path = arg[1] or DEFAULT
local ok_all = check(path)
if ok_all and not check(path, "missing_icon") then ok_all = false end
if ok_all and not check(path, "short_write") then ok_all = false end
if ok_all and not check(path, "stale_response") then ok_all = false end
if not ok_all then os.exit(1) end
