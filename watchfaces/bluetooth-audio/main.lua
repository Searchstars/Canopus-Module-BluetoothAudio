-- One-shot Canopus module installer watchface for the Bluetooth audio module.
-- Opening it stages its signed receipt and ELF, sends CPC2 INSTALL, and performs
-- no activation. On success the supervisor queues a Manager notification and
-- asks the firmware watchface manager to switch away and delete this watchface.
-- Mirrors watchfaces/canopus_hello/main.lua from the Canopus framework.
local lvgl = require("lvgl")

local TOKEN = "bluetooth_audio"
local DEVICE_PATH = "/dev/canopus"
local RECEIPT_RESOURCE = SCRIPT_PATH .. "receipt.bin"
local MODULE_RESOURCE = SCRIPT_PATH .. "module.bin"
local LONG_AUDIO_RESOURCE = SCRIPT_PATH .. "long_test_audio_stream.bin"
local LONG_AUDIO_PATH = "/data/canopus/tmp_btaudio_module_long_audio_test.mp3"
local APP_ICON_RESOURCE = SCRIPT_PATH .. "appicon_headphones.bin"
local APP_ICON_PATH = "/data/canopus/appicon_headphones.bin"
local INBOX = "/data/canopus/inbox/"
local RECEIPT_PATH = INBOX .. TOKEN .. ".cmi"
local MODULE_PATH = INBOX .. TOKEN .. ".ko"
local CPC2_MAGIC = 0x43504332
local CPC1_MAGIC = 0x43504331
local CPS1_MAGIC = 0x43505331
local CMD_INSTALL = 2
local SUP_CMD_QUERY = 0x43510001
local DIAG_QUERY_MAGIC = 0x43514431
local HEADER_SIZE = 36
local RESULT_COMPLETED = 5

local rootbase = lvgl.Object(nil, {
    w = lvgl.HOR_RES(), h = lvgl.VER_RES(), bg_color = 0x07111F,
    bg_opa = lvgl.OPA(100), border_width = 0,
})
local root = lvgl.Object(rootbase, {
    w = 336, h = 480, bg_color = 0x07111F, bg_opa = lvgl.OPA(100),
    border_width = 0, pad_all = 0, align = lvgl.ALIGN.CENTER,
})
local title = lvgl.Label(root, {
    text = "Bluetooth Audio", text_color = 0xFFFFFF,
    align = { type = lvgl.ALIGN.TOP_MID, x_ofs = 0, y_ofs = 52 },
})
local status = lvgl.Label(root, {
    text = "Preparing signed module…", text_color = 0xBFD9FF,
    width = 300, height = 220,
    align = { type = lvgl.ALIGN.TOP_MID, x_ofs = 0, y_ofs = 106 },
})

local function read_all(path)
    local file = io.open(path, "rb")
    if not file then return nil end
    local content = file:read("*a")
    file:close()
    return content
end

local function write_all(path, content)
    local file = io.open(path, "wb")
    if not file then return false end
    local call_ok, result = pcall(file.write, file, content)
    local close_ok, close_result = pcall(file.close, file)
    return call_ok and result ~= nil and close_ok and close_result ~= nil
end

local function word(value)
    value = math.floor(value)
    return string.char(value % 0x100, math.floor(value / 0x100) % 0x100,
        math.floor(value / 0x10000) % 0x100,
        math.floor(value / 0x1000000) % 0x100)
end

local function half(value)
    return string.char(value % 0x100, math.floor(value / 0x100) % 0x100)
end

local function u16(data, offset)
    local a, b = data:byte(offset + 1, offset + 2)
    if not b then return nil end
    return a + b * 0x100
end

local function u32(data, offset)
    local a, b, c, d = data:byte(offset + 1, offset + 4)
    if not d then return nil end
    return a + b * 0x100 + c * 0x10000 + d * 0x1000000
end

local function stage_app_icon()
    local content = read_all(APP_ICON_RESOURCE)
    if type(content) ~= "string" or #content < 12
        or content:byte(1) ~= 0x19 or content:byte(2) ~= 0x10
        or u16(content, 4) == nil or u16(content, 6) == nil
        or u16(content, 8) ~= u16(content, 4) * 4
        or u16(content, 10) ~= 0
        or #content ~= 12 + u16(content, 4) * u16(content, 6) * 4 then
        return false, "Missing or invalid headphones icon"
    end
    local probe = io.open(APP_ICON_PATH, "wb")
    if probe then probe:close() else
        os.execute("mkdir /data/canopus")
    end
    if not write_all(APP_ICON_PATH, content) then
        return false, "Cannot stage headphones icon"
    end
    if read_all(APP_ICON_PATH) ~= content then
        return false, "Headphones icon verification failed"
    end
    return true
end

local function fail(message)
    status:set { text = "Install failed\n\n" .. tostring(message)
        .. "\n\nThis installer was kept for diagnostics." }
end

local function stage_files()
    local receipt = read_all(RECEIPT_RESOURCE)
    local module = read_all(MODULE_RESOURCE)
    if type(receipt) ~= "string" or #receipt ~= 256
        or u32(receipt, 0) ~= 0x31494D43 then
        return false, "Missing or invalid signed receipt"
    end
    if type(module) ~= "string" or #module < 512 or #module > 262144
        or module:sub(1, 4) ~= "\127ELF" then
        return false, "Missing or invalid ARM module"
    end
    local long_audio = io.open(LONG_AUDIO_RESOURCE, "rb")
    if not long_audio then
        return false, "Missing long MP3 test resource"
    end
    local long_audio_prefix = long_audio:read(2)
    long_audio:close()
    local first, second
    if long_audio_prefix then first, second = long_audio_prefix:byte(1, 2) end
    if first ~= 0xFF or not second or second < 0xE0 then
        return false, "Invalid long MP3 test resource"
    end
    local icon_ok, icon_error = stage_app_icon()
    if not icon_ok then return false, icon_error end
    local probe = io.open(RECEIPT_PATH, "wb")
    if probe then probe:close() else
        os.execute("mkdir /data/canopus")
        os.execute("mkdir /data/canopus/inbox")
    end
    if not write_all(RECEIPT_PATH, receipt) then
        return false, "Cannot stage receipt"
    end
    if not write_all(MODULE_PATH, module) then
        return false, "Cannot stage module"
    end
    os.execute("cp " .. LONG_AUDIO_RESOURCE .. " " .. LONG_AUDIO_PATH)
    local staged_audio = io.open(LONG_AUDIO_PATH, "rb")
    if not staged_audio then
        return false, "Cannot stage long MP3 test audio"
    end
    local staged_audio_prefix = staged_audio:read(2)
    staged_audio:close()
    local staged_first, staged_second
    if staged_audio_prefix then
        staged_first, staged_second = staged_audio_prefix:byte(1, 2)
    end
    if staged_first ~= 0xFF or not staged_second or staged_second < 0xE0 then
        return false, "Invalid staged long MP3 test audio"
    end
    -- Byte-for-byte readback. The firmware's os.execute cannot run `chmod`
    -- (installer versions without it staged and installed cleanly on-device),
    -- so do not gate on it: the supervisor reads the staged files in the same
    -- native context and needs no extra permission grant.
    if read_all(RECEIPT_PATH) ~= receipt or read_all(MODULE_PATH) ~= module then
        return false, "Staged file verification failed"
    end
    return true
end

local function supervisor_error()
    local query = word(CPC1_MAGIC) .. word(SUP_CMD_QUERY)
        .. word(DIAG_QUERY_MAGIC) .. word(0)
    local device = io.open(DEVICE_PATH, "wb")
    if not device then return nil end
    local ok, result = pcall(device.write, device, query)
    pcall(device.close, device)
    if not ok or result == nil
        or (type(result) == "number" and result ~= #query) then
        return nil
    end
    device = io.open(DEVICE_PATH, "rb")
    if not device then return nil end
    local status_record = device:read(384)
    pcall(device.close, device)
    if type(status_record) ~= "string" or #status_record ~= 384
        or u32(status_record, 0) ~= CPS1_MAGIC then
        return nil
    end
    local error = u32(status_record, 32)
    if error >= 0x80000000 then error = error - 0x100000000 end
    return error
end

local function install()
    local ok, message = stage_files()
    if not ok then fail(message) return end
    local payload = TOKEN .. "\0"
    local total = HEADER_SIZE + #payload
    local request = word(CPC2_MAGIC) .. half(HEADER_SIZE) .. half(1)
        .. half(1) .. half(0) .. word(total) .. word(CMD_INSTALL)
        .. word(1) .. word(0) .. word(0) .. word(#payload) .. payload
    local device = io.open(DEVICE_PATH, "wb")
    if not device then fail("Canopus Manager is not installed") return end
    local write_ok, write_result, write_error = pcall(device.write, device, request)
    local close_ok, close_result = pcall(device.close, device)
    -- Firmware bindings may return a byte count; stock Lua returns the file
    -- handle on a complete write and nil on an I/O failure. Enforce exactness
    -- whenever the binding exposes a count without rejecting stock semantics.
    local short_write = type(write_result) == "number" and write_result ~= #request
    if not write_ok or write_result == nil or short_write
        or not close_ok or close_result == nil then
        fail(write_error or "Supervisor write failed") return
    end

    -- If self-removal succeeds this script may stop before the read. If deletion
    -- is refused (for example this is the last watchface), the response remains
    -- available and this page explains that installation still completed.
    local response_file = io.open(DEVICE_PATH, "rb")
    if not response_file then
        fail("Cannot read supervisor response")
        return
    end
    local response = response_file:read(HEADER_SIZE)
    response_file:close()
    if type(response) ~= "string" or #response ~= HEADER_SIZE
        or u32(response, 0) ~= CPC2_MAGIC
        or u16(response, 4) ~= HEADER_SIZE
        or u16(response, 6) ~= 2
        or u16(response, 8) ~= 1
        -- Accept newer ABI 1.x responses. The request uses the compatible 1.0
        -- prefix, while the supervisor may report a newer 1.x minor.
        or u32(response, 12) ~= HEADER_SIZE
        or u32(response, 16) ~= CMD_INSTALL
        or u32(response, 20) ~= 1
        or u32(response, 24) ~= 0
        or u32(response, 32) ~= 0 then
        fail("Invalid supervisor response") return
    end
    local result = u32(response, 28)
    if result ~= RESULT_COMPLETED then
        local error = supervisor_error()
        local detail = error and " (error " .. tostring(error) .. ")" or ""
        fail("Supervisor result " .. tostring(result) .. detail)
        return
    end
    status:set { text = "Installed — disabled by default.\n\n"
        .. "Open Canopus Manager to review and enable the module.\n\n"
        .. "Automatic watchface removal was refused; remove this installer manually." }
end

local ran = false
local function run_once()
    if ran then return end
    ran = true
    install()
end

run_once()
