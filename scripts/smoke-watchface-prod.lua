-- Exercises firmware selection and fail-closed installation for the production
-- multi-target watchface without touching a device or generated payloads.
local DEFAULT = "watchfaces/bluetooth-audio-prod/main.lua"
local TOKEN = "bluetooth_audio"
local TEMP = "/data/bluetooth-audio-installer-version.tmp"
local TARGETS = {
    ["3.101.036"] = {
        id = "xiaomi-band-10-pro-3.101.036",
        firmware = "662d67f5e247e31e194d3161024890ba93b9d29d70b290fadb9aac8ce8ec3c81",
    },
    ["3.101.043"] = {
        id = "xiaomi-band-10-pro-3.101.043",
        firmware = "519307675665e4866d722a8119a98589c397b614ac3294cb87bfc86de45756ec",
    },
}

local lvgl = {}
lvgl.HOR_RES = function() return 336 end
lvgl.VER_RES = function() return 480 end
lvgl.OPA = function(value) return value end
lvgl.ALIGN = { CENTER = 1, TOP_MID = 2 }
local created = {}
local object_mt = {}
function object_mt:set(props) self._last_set = props; return self end
function lvgl.Object(parent, props)
    local object = setmetatable({ _parent = parent, _props = props }, { __index = object_mt })
    table.insert(created, object)
    return object
end
function lvgl.Label(parent, props)
    local object = setmetatable({ _parent = parent, _props = props }, { __index = object_mt })
    table.insert(created, object)
    return object
end

_G.SCRIPT_PATH = "/fake/"
package.loaded["lvgl"] = lvgl

local original_io_open = io.open
local original_os_execute = os.execute
local original_os_remove = os.remove

local function le32(value)
    return string.char(value % 256, math.floor(value / 256) % 256,
        math.floor(value / 65536) % 256, math.floor(value / 16777216) % 256)
end

local function icon_fixture()
    return string.char(0x19, 0x10, 0, 0, 2, 0, 2, 0, 8, 0, 0, 0)
        .. string.rep("\0", 16)
end

local function pad(value, length)
    assert(#value <= length)
    return value .. string.rep("\0", length - #value)
end

local function from_hex(value)
    return (value:gsub("..", function(pair) return string.char(tonumber(pair, 16)) end))
end

local function module_payload()
    return "\127ELF" .. string.char(1, 1, 1) .. string.rep("\0", 9)
        .. string.char(1, 0, 40, 0) .. string.rep("\0", 492)
end

local function receipt_payload(target)
    local module = module_payload()
    local header = le32(0x31494D43) .. le32(1) .. le32(256) .. le32(0)
        .. le32(1) .. le32(1) .. le32(#module) .. le32(0)
    return header .. pad(TOKEN, 32) .. pad(target.id, 48)
        .. from_hex(target.firmware) .. string.rep("\0", 112)
end

local function resource_path(target, extension)
    return "/fake/bluetooth-audio-" .. target.id .. extension
end

local function installer_io(version, fault)
    local module = module_payload()
    local files = {}
    for _, target in pairs(TARGETS) do
        files[resource_path(target, ".bin")] = module
        files[resource_path(target, ".cmi.bin")] = receipt_payload(target)
    end
    local selected = TARGETS[version]
    if selected and fault == "missing_module" then
        files[resource_path(selected, ".bin")] = nil
    elseif selected and fault == "missing_receipt" then
        files[resource_path(selected, ".cmi.bin")] = nil
    elseif selected and fault == "wrong_pair" then
        local other = TARGETS[version == "3.101.043" and "3.101.036" or "3.101.043"]
        files[resource_path(selected, ".cmi.bin")] = receipt_payload(other)
    elseif selected and fault == "invalid_module" then
        files[resource_path(selected, ".bin")] = "not an ELF module"
    end

    local state = { files = files, opened = {}, commands = {}, request = nil }
    files["/fake/appicon_headphones.bin"] = icon_fixture()
    if fault == "missing_icon" then
        files["/fake/appicon_headphones.bin"] = nil
    end
    local function open(path, mode)
        state.opened[path] = (state.opened[path] or 0) + 1
        if path == "/dev/canopus" then
            if mode == "wb" then
                return {
                    write = function(_, data)
                        state.request = data
                        if fault == "short_write" then return #data - 1 end
                        return #data
                    end,
                    close = function() return true end,
                }
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
        if mode == "rb" or mode == "r" then
            local content = state.files[path]
            if content == nil then return nil end
            return {
                read = function() return content end,
                close = function() return true end,
            }
        end
        if mode == "wb" then
            return {
                write = function(_, data) state.files[path] = data; return true end,
                close = function() return true end,
            }
        end
        return nil
    end
    local function execute(command)
        table.insert(state.commands, command)
        if command:match("^getprop ") then
            if fault == "getprop_failed" then return false end
            state.files[TEMP] = tostring(version or "") .. "\n"
        end
        return true
    end
    local function remove(path)
        state.files[path] = nil
        return true
    end
    return open, execute, remove, state
end

local function status_contains(pattern)
    for _, object in ipairs(created) do
        local current = object._last_set and object._last_set.text
        local initial = object._props and object._props.text
        if type(current) == "string" and current:match(pattern) then return true end
        if type(initial) == "string" and initial:match(pattern) then return true end
    end
    return false
end

local function contains_test_audio_reference(state)
    for path in pairs(state.opened) do
        local lower = path:lower()
        if lower:match("long_test_audio") or lower:match("tmp_btaudio")
            or lower:match("%.mp3") then return true end
    end
    for _, command in ipairs(state.commands) do
        local lower = command:lower()
        if lower:match("long_test_audio") or lower:match("tmp_btaudio")
            or lower:match("%.mp3") then return true end
    end
    return false
end

local function check(path, name, version, fault, expect_request)
    created = {}
    local open, execute, remove, state = installer_io(version, fault)
    io.open = open
    os.execute = execute
    os.remove = remove
    local ok, error_message = pcall(dofile, path)
    io.open = original_io_open
    os.execute = original_os_execute
    os.remove = original_os_remove
    if not ok then
        print("LOAD FAIL:", name, error_message)
        return false
    end
    if contains_test_audio_reference(state) then
        print("TEST AUDIO REFERENCE FAIL:", name)
        return false
    end

    local target = TARGETS[version]
    if fault == "getprop_failed" then target = nil end
    if target then
        if not state.opened[resource_path(target, ".bin")]
            or not state.opened[resource_path(target, ".cmi.bin")] then
            print("TARGET SELECTION FAIL:", name)
            return false
        end
        for candidate_version, candidate in pairs(TARGETS) do
            if candidate_version ~= version
                and (state.opened[resource_path(candidate, ".bin")]
                    or state.opened[resource_path(candidate, ".cmi.bin")]) then
                print("CROSS-TARGET RESOURCE FAIL:", name)
                return false
            end
        end
    else
        for _, candidate in pairs(TARGETS) do
            if state.opened[resource_path(candidate, ".bin")]
                or state.opened[resource_path(candidate, ".cmi.bin")] then
                print("UNSUPPORTED RESOURCE ACCESS FAIL:", name)
                return false
            end
        end
    end

    if expect_request then
        local suffix = TOKEN .. "\0"
        if type(state.request) ~= "string" or state.request:sub(1, 4) ~= "2CPC"
            or state.request:byte(17) ~= 2
            or state.request:sub(-#suffix) ~= suffix
            or state.files["/data/canopus/inbox/" .. TOKEN .. ".cmi"] == nil
            or state.files["/data/canopus/inbox/" .. TOKEN .. ".ko"] == nil
            or state.files["/data/canopus/appicon_headphones.bin"] ~= icon_fixture() then
            print("INSTALL FLOW FAIL:", name)
            return false
        end
    elseif state.request ~= nil then
        print("FAIL-CLOSED REQUEST FAIL:", name)
        return false
    end

    local should_succeed = fault == nil and target ~= nil
    if should_succeed and not status_contains("Installed") then
        print("SUCCESS STATUS FAIL:", name)
        return false
    end
    if not should_succeed and target == nil and not status_contains("not supported") then
        print("UNSUPPORTED STATUS FAIL:", name)
        return false
    end
    if not should_succeed and target ~= nil and not status_contains("Install failed") then
        print("DIAGNOSTIC STATUS FAIL:", name)
        return false
    end
    print("production watchface OK:", name)
    return true
end

local path = arg[1] or DEFAULT
local cases = {
    { "firmware-036", "3.101.036", nil, true },
    { "firmware-043", "3.101.043", nil, true },
    { "unsupported", "3.101.999", nil, false },
    { "malformed-version", "invalid", nil, false },
    { "getprop-failed", "3.101.043", "getprop_failed", false },
    { "missing-module", "3.101.043", "missing_module", false },
    { "missing-receipt", "3.101.036", "missing_receipt", false },
    { "cross-target-receipt", "3.101.043", "wrong_pair", false },
    { "invalid-module", "3.101.036", "invalid_module", false },
    { "missing-icon", "3.101.043", "missing_icon", false },
    { "short-supervisor-write", "3.101.043", "short_write", true },
    { "stale-supervisor-response", "3.101.036", "stale_response", true },
}
for _, case in ipairs(cases) do
    if not check(path, case[1], case[2], case[3], case[4]) then os.exit(1) end
end
