# Noia DNF

Noia DNF 是一款用 Rust 和 GPUI Kit 构建的 Windows x64 桌面工具。它支持在 DNF 前台按住选定的按键连发，松开即停。可以在键盘布局上选择按键，并通过滑块或手动输入调整间隔，从 5 到 1000 ms，默认 50 ms。应用提供简体中文和英文界面，支持系统托盘和单实例运行。

## 安装和使用

从 [GitHub Releases](https://github.com/ZDYoung0519/noia-dnf/releases) 下载安装器，或解压便携版 ZIP 后运行 `noia-dnf.exe`。安装器支持自定义目录和桌面快捷方式。应用启动时会请求管理员权限，以便向同样以高权限运行的游戏发送按键。语言和连发设置保存在可执行文件旁的 `settings/`。

侧边栏左下角的版本号会打开关于页，显示 Logo、当前版本和最近的更新日志。启动时会自动检查更新。检查期间显示转圈，发现新版后自动下载，圆环显示实际下载百分比。校验通过后，点击 **等待升级** 并确认即可重启安装。选择 **稍后** 可以继续使用当前版本。点击 **最新版本** 可再次检查，网络或校验失败时可点击 **重试更新**。

## 本地开发

在 Windows x64 上安装 Rust 的 `stable-x86_64-pc-windows-msvc` 工具链、Visual Studio C++ Build Tools 和 Windows SDK。制作安装器还需要 NSIS 3。

```powershell
cargo run --locked -p noia-dnf
cargo test --workspace --locked --release
```

主程序和 GPUI 界面在 `src/`。独立库 `crates/noia-updater/` 负责读取清单、比较语义版本、下载并校验安装器，以及启动升级。工作区不依赖其他 Noia 项目的源码。

## 配置更新源

仓库根目录的 `.env` 指定 GitHub Releases 地址。构建环境中同名变量优先，适合 CI 和 fork。地址在构建时写入客户端，打包脚本也用它生成附件链接。

```dotenv
NOIA_RELEASE_URL=https://github.com/ZDYoung0519/noia-dnf/releases
```

客户端读取 `<NOIA_RELEASE_URL>/latest/download/latest.json`。清单使用 `schema_version: 1`，包含版本、双语更新日志和 `assets.windows-x86_64` 下的安装器 URL、字节数及 SHA-256。客户端只接受配置仓库中对应版本标签的安装器。安装版原地升级，便携版只替换当前目录的主程序，两种方式均保留 `settings/`。

## 打包和发布

第一次发布前安装 GitHub CLI，运行 `gh auth login` 登录对发布仓库有写入权限的账号。CI 可以设置 `GH_TOKEN`。

先在根目录 `change_log.json` 的 `releases` 数组开头写好新版本的中英文更新内容，保留历史条目。仓库已包含 `0.1.0` 和预写的 `0.1.1`。发布时只需运行：

```powershell
.\scripts\build-release.ps1
```

根据提示选 Patch、Minor、Major，也可以输入完整版本号。选择 `0.1.1` 时，脚本先核对该条日志，再更新主程序的 `Cargo.toml` 和 `Cargo.lock`。它将**全部未被 Git 忽略的项目改动**（含已暂存、未暂存、新增及删除）一起提交。测试和打包成功后，脚本推送当前分支到 `origin`，为发布提交创建 GitHub 标签，并核验安装器、便携版和清单的 SHA-256 后将草稿发布为 **Latest**。`latest.json` 与 GitHub Release 正文均使用所选版本的日志。

| 命令 | 用途 |
| --- | --- |
| `.\scripts\build-release.ps1 -Version 0.1.1` | 直接选择已写好日志的新版本 |
| `.\scripts\build-release.ps1 -Increment 1` | 自动选择下一个 Patch；2 为 Minor，3 为 Major |
| `.\scripts\build-release.ps1 -Resume` | 版本已提交后，按当前提交重试构建、推送或继续上传草稿 |
| `.\scripts\build-release.ps1 -BuildOnly` | 本地测试及打包当前版本，不修改版本、不提交、不上传 |

发布命令使用 `origin` 同步源码，并使用 `.env` 指向的 GitHub Releases 仓库发布。该仓库需要包含对应的源码提交。需要指定 NSIS 路径时，可传入 `-MakeNsis 'C:\Tools\NSIS\makensis.exe'`。单独制作安装包也可以使用 `.\scripts\build-installer.ps1`。

| 默认输出路径 | 用途 |
| --- | --- |
| `target/installer/Noia-DNF-<版本>-windows-x64-setup.exe` | 安装和应用内升级 |
| `target/installer/latest.json` | 客户端更新清单 |
| `target/Noia-DNF-windows-x64.zip` | 便携版 |

发布后可查看[线上更新清单](https://github.com/ZDYoung0519/noia-dnf/releases/latest/download/latest.json)。首个支持更新器的版本需手动安装一次，后续版本可在客户端升级。
