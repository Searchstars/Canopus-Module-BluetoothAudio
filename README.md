# Canopus-Module-Bluetooth-Audio

一个Canopus模块，为不支持连接蓝牙耳机的小米Vela穿戴设备补全蓝牙音频能力。

## 构建

```sh
git clone https://github.com/AstralSightStudios/Canopus
git clone https://github.com/Searchstars/Canopus-Module-Blutooth-Audio

cargo test --workspace
scripts/build-device.sh

```

注：该模块与Canopus框架属于并行开发关系，我希望利用在开发该模块过程中所获得的经验来完善Canopus框架，因此对Canopus框架的引用属于本地引用。在构建时，请确保Canopus框架项目文件夹与该项目文件夹同级。

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

## 验证

```sh
scripts/build-device.sh
scripts/build-install-watchface.sh
```
