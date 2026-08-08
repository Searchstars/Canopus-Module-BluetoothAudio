-- Persistent HCI btsnoop control/export watchface for BluetoothAudio.
-- It mirrors the device-proven Phase 5 flow: enable the stock recorder for one
-- attempt, then disable it and copy the complete snoop directory offline.

local lvgl = require("lvgl")

local SNOOP_SOURCE = "/data/misc/bt/snoop"
local OFFLINELOG = "/data/offlinelog"
local SNOOP_DESTINATION = OFFLINELOG .. "/snoop"
local PROPERTY_SNAPSHOT = OFFLINELOG .. "/.bluetooth_audio_snoop_property"

local function shell_quote(value)
    return "'" .. tostring(value):gsub("'", "'\\''") .. "'"
end

local function run(command)
    print("[bluetooth_audio_snoop] exec: " .. command)
    local ok, why, code = os.execute(command)
    print(string.format("[bluetooth_audio_snoop] result: %s %s %s",
        tostring(ok), tostring(why), tostring(code)))
    return ok == true or ok == 0
end

local function read_all(path)
    local file = io.open(path, "rb")
    if not file then return nil end
    local data = file:read("*a")
    file:close()
    return data
end

local function snoop_enabled()
    if not run("mkdir " .. shell_quote(OFFLINELOG)) then
        -- mkdir returns failure when the directory already exists on some NSH
        -- builds; the subsequent property redirection remains the authority.
    end
    if not run("getprop persist.bluetooth.log.snoop_enable > "
            .. shell_quote(PROPERTY_SNAPSHOT)) then
        return nil
    end
    local data = read_all(PROPERTY_SNAPSHOT)
    run("rm -f " .. shell_quote(PROPERTY_SNAPSHOT))
    if type(data) ~= "string" then return nil end
    return data:match("^%s*1%s*$") ~= nil
end

local function set_snoop_enabled(enabled)
    local value = enabled and 1 or 0
    if not run("setprop persist.bluetooth.log.snoop_enable " .. value) then
        return false
    end
    -- btserver observes this pulse instead of the recorder property itself.
    if not run("setprop persist.bluetooth.log.changed 0") then
        return false
    end
    return run("setprop persist.bluetooth.log.changed 8")
end

local function export_snoop_directory()
    -- Replace only this export. The source directory is never modified.
    run("mkdir " .. shell_quote(OFFLINELOG))
    if not run("rm -rf " .. shell_quote(SNOOP_DESTINATION)) then
        return false
    end
    return run("cp -r " .. shell_quote(SNOOP_SOURCE) .. " "
        .. shell_quote(SNOOP_DESTINATION))
end

local root = lvgl.Object(nil, {
    w = lvgl.HOR_RES(), h = lvgl.VER_RES(), bg_color = 0x07111F,
    bg_opa = lvgl.OPA(100), border_width = 0, pad_all = 12,
})
root:add_flag(lvgl.FLAG.SCROLLABLE)

local title = lvgl.Label(root, {
    text = "Bluetooth HCI Snoop", text_color = 0xFFFFFF,
    align = { type = lvgl.ALIGN.TOP_MID, x_ofs = 0, y_ofs = 20 },
})

local status = lvgl.Label(root, {
    text = "Read stock recorder state…", text_color = 0xBFD9FF,
    w = 300, h = 250,
    align = { type = lvgl.ALIGN.TOP_MID, x_ofs = 0, y_ofs = 66 },
})

local action = lvgl.Object(root, {
    w = 280, h = 54, bg_color = 0x8A6A17, bg_opa = lvgl.OPA(100),
    border_width = 0, radius = 11,
    align = { type = lvgl.ALIGN.BOTTOM_MID, x_ofs = 0, y_ofs = -28 },
})
action:clear_flag(lvgl.FLAG.SCROLLABLE)
action:add_flag(lvgl.FLAG.CLICKABLE)
local action_text = lvgl.Label(action, {
    text = "Enable HCI snoop", text_color = 0xFFFFFF,
    align = { type = lvgl.ALIGN.CENTER, x_ofs = 0, y_ofs = 0 },
})

local exported = false
local busy = false

local function refresh()
    local enabled = snoop_enabled()
    if enabled == nil then
        status:set { text = "Cannot read stock HCI snoop state.\n\n"
            .. "No Bluetooth setting was changed." }
        action_text:set { text = "Retry state check" }
        return nil
    end
    if enabled then
        status:set { text = "Stock HCI snoop is enabled.\n\n"
            .. "Run one BluetoothAudio pairing/connection attempt.\n"
            .. "Immediately return here and press Close + Export.\n\n"
            .. "The recorder must be closed before copying." }
        action_text:set { text = "Close + Export snoop" }
    elseif exported then
        status:set { text = "Snoop was closed and copied to:\n"
            .. SNOOP_DESTINATION .. "\n\n"
            .. "Reboot before recording another attempt." }
        action_text:set { text = "Export complete" }
    else
        status:set { text = "Stock HCI snoop is disabled.\n\n"
            .. "Press Enable, run one pairing attempt, then return and press "
            .. "Close + Export.\n\n"
            .. "Export destination:\n" .. SNOOP_DESTINATION }
        action_text:set { text = "Enable HCI snoop" }
    end
    return enabled
end

action:onClicked(function()
    if busy then return end
    busy = true
    local enabled = snoop_enabled()
    if enabled == nil then
        status:set { text = "Cannot read recorder state.\n"
            .. "No Bluetooth setting was changed." }
    elseif enabled then
        local disabled = set_snoop_enabled(false)
        if disabled then run("sleep 1") end
        local copied = disabled and export_snoop_directory()
        exported = disabled and copied
        if copied then
            status:set { text = "HCI snoop closed and copied.\n\n"
                .. "Complete folder:\n" .. SNOOP_DESTINATION .. "\n\n"
                .. "Export device logs now; do not run another pairing attempt."
            }
            action_text:set { text = "Export complete" }
        elseif disabled then
            status:set { text = "Snoop closed, but folder copy failed.\n\n"
                .. "Preserve device logs and reboot; do not retry pairing." }
            action_text:set { text = "Retry export" }
        else
            status:set { text = "Cannot close stock HCI snoop.\n\n"
                .. "Press again only to retry disabling it." }
            action_text:set { text = "Retry close" }
        end
    elseif exported then
        status:set { text = "This boot's snoop export is complete.\n\n"
            .. "Reboot before recording another attempt." }
    elseif set_snoop_enabled(true) then
        run("sleep 1")
        refresh()
    else
        status:set { text = "Cannot enable stock HCI snoop.\n\n"
            .. "No Bluetooth setting was changed." }
    end
    busy = false
end)

refresh()
