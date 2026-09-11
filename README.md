# Canopus-Module-Bluetooth-Audio

一个Canopus模块，为不支持连接蓝牙耳机的小米Vela穿戴设备补全蓝牙音频能力。

## 兼容性
项目在固件已有的蓝牙L2CAP栈上重建了A2DP+AVDTP音频栈，构建一个IO设备接收mp3音频流，实时解码并编码成SBC数据包发送给已连接的耳机。

由于硬件特性限制，每次连接耳机时，都**必须将耳机置于配对模式**。

目前支持的设备/固件版本如下（“编译支持”不等于设备验证或生产批准）：
|设备型号|固件版本|状态/备注|
|-|-|-|
|小米手环10 Pro|`3.101.036`|trusted build target|
|小米手环10 Pro|`3.101.043`|可构建；device gate pending|
|小米手环11|`4.100.139`|独立 Rust 后端已接入，固件/编译后 ABI 测试通过；可安装启用，耳机连接与播放待实机验证|

既有设备测试的耳机/音响兼容情况如下（尚未在 Band 11 `.139` 复测）：

|耳机/音响型号|可用性|备注|
|-|-|-|
|REDMI 头戴式耳机|✅|工作良好|
|联想服务 LE202 耳机|✅|工作良好|
|Xiaomi 随身蓝牙音箱|✅|工作良好|
|小米手环 多功能桌搭|✅|工作良好|
|REDMI Buds 8 Pro|✅|多次进入配对模式可能触发bug导致无法被扫描到，此时持续按住配对键恢复出厂设置即可|
|AirPods Pro 3|❌|可以连接但没有声音，疑似请求音质过高导致|
|猫王·小王子音箱|❌|蓝牙版本过旧，无法连接|

## 构建

**IMPORTANT**: 构建所需的Canopus框架本体由于包含一些保密内容，综合安全性与合法性考虑，暂不做开源，仅向AstroBox项目核心开发人员开放。

```sh
git clone https://github.com/AstralSightStudios/Canopus
git clone https://github.com/Searchstars/Canopus-Module-BluetoothAudio

cargo test --workspace
scripts/build-device.sh

```

注：该模块与Canopus框架属于并行开发关系，我希望利用在开发该模块过程中所获得的经验来完善Canopus框架，因此对Canopus框架的引用属于本地引用。在构建时，请确保Canopus框架项目文件夹与该项目文件夹同级。依赖路径使用相对路径，不绑定某台开发机的绝对目录；`CANOPUS_ROOT` 可覆盖构建脚本使用的 CLI、SDK 头文件和 target-pack checkout。

### Target 选择

Rust 私有 ABI 由 `canopus-target-private` facade 的互斥 `target-*` feature 选择。模块构建脚本读取 `targets/<target-id>.env`，由该文件映射 Cargo feature、Rust target triple、CPU 与 loader 大小限制。当前默认值为：

```sh
CANOPUS_TARGET=xiaomi-band-10-pro-3.101.036 scripts/build-device.sh
# 构建 Canopus.toml include 的全部 target（036、043、139 以及两个 compile-only static candidate）：
scripts/build-targets.sh
```

产物位于 `build/<target-id>/bluetooth-audio.elf`。增加新 target 时，需要先添加独立 target pack 和经过验证的 private ABI backend，再添加对应 `.env` profile；Bluetooth/AVDTP 模块逻辑、descriptor target ID 和 C constructor 不应出现新的固件地址或 target 常量。缺失或未知 target 会在调用私有 ABI 前 fail closed。

交叉编译需要在 **nightly** 工具链（`cargo +nightly`）下运行，请确保已安装该工具链（`rustup toolchain install nightly`）。编译出的交叉二进制文件通过以下两个手段来控制体积：

* `-C symbol-mangling-version=hashed`（需要配合 `-Z unstable-options`）：缩短占用 `.symtab`/`.strtab` 主要空间的长 Rust 符号和段名称；
* `-Z function-sections=no`：合并约 890 个独立函数段，从而压缩 `.shstrtab` 和段标头（section headers）。

`RUSTFLAGS` 会替换 `.cargo/config.toml` 中的 `[target.*]` 标志，因此脚本中重复指定了 `-C panic=abort -C target-cpu=cortex-m33`。在完成 `ld.lld -r` 后，`rust-objcopy --remove-section=.llvmbc --strip-debug` 会清理未消耗的薄 LTO（thin-LTO）字节码和调试元数据。可以通过 `NIGHTLY_CARGO` 覆盖工具链，通过 `RUST_OBJCOPY` 覆盖 objcopy 二进制文件，以及通过 `CANOPUS_ROOT` 覆盖框架根目录。

## 设备端安装（通过表盘）

该模块利用 Canopus 的“安装表盘（install watchface）”概念（参见框架中的 `watchfaces/canopus_hello`），打包为一个一次性的安装器表盘。

```sh
scripts/build-install-watchface.sh

```

该命令会执行交叉编译、运行 Canopus ELF 验证器、使用本地开发密钥对 CMI1 凭证进行签名、对 Lua 安装器进行smoke test，并将有效载荷暂存至 `watchfaces/bluetooth-audio/`

Prod 按设备分别输出安装表盘，同一设备目录内可包含多个固件版本，上机后自动选择匹配版本：

```sh
scripts/build-install-watchface-prod.sh
```

默认包含 10 Pro `.036`、`.043` 与 Band 11 `.139`；也可只构建 Band 11：

```sh
scripts/build-install-watchface-prod.sh xiaomi-band-11
```

产物分别在 `watchfaces/bluetooth-audio-prod/xiaomi-band-10-pro/`（`.036`、`.043`）与
`watchfaces/bluetooth-audio-prod/xiaomi-band-11/`（`.139`）。将对应设备目录交给打包器，
目录根部仅一个 `main.lua` 与 `.bin` 资源；子目录 `build/`、`docs/` 不参与打包。
也可以传入一个或多个完整 target ID，或设置 `CANOPUS_DEVICE` / `CANOPUS_TARGET`；
脚本参数优先，其次 `CANOPUS_DEVICE`。选择单个版本会重新生成该设备包，其他设备目录保留。

须先更新 Canopus 框架安装表盘；若旧 Supervisor 已常驻，重启后运行新版框架安装器。
框架准备 `/data/canopus/inbox` 并注册 `/canopus/install`。外部 Lua 仅用普通 `io.open`
提交签名安装请求，不包含 execute 恢复或 debug 操作。安装结果保持禁用。
打包时校验凭证签名、目标和模块哈希，逐步显示写入及回读状态。

Band 11 框架管理器已有用户实机成功反馈，但本模块的 `.139` Rust 后端
`canopus_identity_guard()` 仍返回 `-38`，蓝牙和音频服务尚未恢复；
此安装包只能用于安装流程验证，不能作为可用的蓝牙音频模块。
本轮已核验并修正框架内八个 `.139` Bluelet 候选，新增真实固件 ARM 测试；
定时器参数、取消所有权和 L2CAP CID 偏移均与 `.043` 不同，不能仅替换地址。
详见同级框架仓库的 `docs/band11-bluetooth-139.md`。

## 验证

```sh
scripts/build-device.sh
scripts/build-install-watchface.sh
```
