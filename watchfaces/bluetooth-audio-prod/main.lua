-- Multi-target production installer for the Bluetooth audio module.
-- The matching signed payload is selected from the exact firmware version;
-- unsupported firmware fails before anything is staged or sent to Supervisor.
local lvgl = require("lvgl")

local TOKEN = "bluetooth_audio"
local DEVICE_PATH = "/dev/canopus"
local FIRMWARE_VERSION_PROPERTY = "ro.build.version"
local FIRMWARE_VERSION_OUTPUT = "/data/bluetooth-audio-installer-version.tmp"
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

local TARGETS = {
    ["3.101.030"] = { id = "xiaomi-band-10-pro-3.101.030" },
    ["3.101.036"] = { id = "xiaomi-band-10-pro-3.101.036" },
}

local rootbase = lvgl.Object(nil, {
    w = lvgl.HOR_RES(), h = lvgl.VER_RES(), bg_color = 0x07111F,
    bg_opa = lvgl.OPA(100), border_width = 0,
})
local root = lvgl.Object(rootbase, {
    w = 336, h = 480, bg_color = 0x07111F, bg_opa = lvgl.OPA(100),
    border_width = 0, pad_all = 0, align = lvgl.ALIGN.CENTER,
})
lvgl.Label(root, {
    text = "Bluetooth Audio", text_color = 0xFFFFFF,
    align = { type = lvgl.ALIGN.TOP_MID, x_ofs = 0, y_ofs = 52 },
})
local status = lvgl.Label(root, {
    text = "Detecting firmware…", text_color = 0xBFD9FF,
    width = 300, height = 220,
    align = { type = lvgl.ALIGN.TOP_MID, x_ofs = 0, y_ofs = 106 },
})

local function shell_quote(value)
    return "'" .. tostring(value):gsub("'", "'\\''") .. "'"
end

local function run(command)
    print("[bluetooth-audio-prod] exec: " .. command)
    local ok = os.execute(command)
    return ok == true or ok == 0
end

local function read_all(path, mode)
    local file = io.open(path, mode or "rb")
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

local function fixed_string(data, offset, length)
    local value = data:sub(offset + 1, offset + length)
    local terminator = value:find("\0", 1, true)
    if terminator then value = value:sub(1, terminator - 1) end
    return value
end

local function detect_firmware_version()
    local command = string.format("getprop %s > %s",
        shell_quote(FIRMWARE_VERSION_PROPERTY),
        shell_quote(FIRMWARE_VERSION_OUTPUT))
    local command_ok = run(command)
    local raw
    if command_ok then raw = read_all(FIRMWARE_VERSION_OUTPUT, "r") end
    if type(os.remove) == "function" then
        pcall(os.remove, FIRMWARE_VERSION_OUTPUT)
    end
    if not command_ok or type(raw) ~= "string" then return nil end
    local version = raw:match("^%s*(.-)%s*$")
    if not version or not version:match("^%d+%.%d+%.%d+$") then return nil end
    return version
end

local function fail(message)
    status:set { text = "Install failed\n\n" .. tostring(message)
        .. "\n\nThis installer was kept for diagnostics.", text_color = 0xFF9A9A }
end

local function resource_paths(target)
    local stem = SCRIPT_PATH .. "bluetooth-audio-" .. target.id
    return stem .. ".cmi.bin", stem .. ".bin"
end

local function stage_files(target, receipt_resource, module_resource)
    local receipt = read_all(receipt_resource)
    local module = read_all(module_resource)
    if type(receipt) ~= "string" or #receipt ~= 256
        or u32(receipt, 0) ~= 0x31494D43
        or u32(receipt, 4) ~= 1
        or u32(receipt, 8) ~= 256
        or fixed_string(receipt, 32, 32) ~= TOKEN
        or fixed_string(receipt, 64, 48) ~= target.id then
        return false, "Missing or invalid target receipt"
    end
    if type(module) ~= "string" or #module < 512 or #module > 262144
        or module:sub(1, 4) ~= "\127ELF"
        or module:byte(5) ~= 1 or module:byte(6) ~= 1 or module:byte(7) ~= 1
        or u16(module, 16) ~= 1 or u16(module, 18) ~= 40 then
        return false, "Missing or invalid relocatable ARM module"
    end
    if u32(receipt, 24) ~= #module then
        return false, "Receipt artifact size mismatch"
    end
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

local function install(target, receipt_resource, module_resource)
    local ok, message = stage_files(target, receipt_resource, module_resource)
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
    local short_write = type(write_result) == "number" and write_result ~= #request
    if not write_ok or write_result == nil or short_write
        or not close_ok or close_result == nil then
        fail(write_error or "Supervisor write failed") return
    end

    local response_file = io.open(DEVICE_PATH, "rb")
    if not response_file then fail("Cannot read supervisor response") return end
    local response = response_file:read(HEADER_SIZE)
    response_file:close()
    if type(response) ~= "string" or #response ~= HEADER_SIZE
        or u32(response, 0) ~= CPC2_MAGIC
        or u16(response, 4) ~= HEADER_SIZE
        or u16(response, 6) ~= 2
        or u16(response, 8) ~= 1
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
        .. "Firmware " .. target.id:match("(%d+%.%d+%.%d+)$"),
        text_color = 0x8FF0A4 }
end

local firmware_version = detect_firmware_version()
local target = firmware_version and TARGETS[firmware_version] or nil
if not target then
    status:set { text = "Firmware version not supported\n"
        .. tostring(firmware_version or "Unknown"), text_color = 0xFF9A9A }
    return
end

local receipt_resource, module_resource = resource_paths(target)
local ran = false
local function run_once()
    if ran then return end
    ran = true
    install(target, receipt_resource, module_resource)
end
run_once()
