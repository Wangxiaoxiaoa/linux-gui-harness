# Wayland 后端集成设计（feat/wayland）

> 状态：草案，待评审。本文档批准后作为 `feat/wayland` 分支的实现依据。
> 原型验证：`~/桌面/wayland-probe`（全部关键假设已实测，证据见附录 A）。

## 1. 背景与目标

本项目当前只支持 X11（Xvfb + openbox 沙盒）。目标：增加 Wayland 后端，
使 AI 智能体可以在 **Wayland 协议的隔离图形世界** 中操作应用——同时保持
X11 路径零改动。

原型（wsp）已实测验证的核心事实：

| 事实 | 数据 |
|---|---|
| 沙盒组合器可多开、可隐形 | 5 个并行 sway（headless），互不可见 |
| 启动/销毁毫秒级 | 0.365s / 24–66ms |
| 组合器成本 | PSS ~15MB，空闲 CPU ~0.4% |
| 同一工具面覆盖双协议 | 37 个工具无差别工作 |
| 原生嵌窗可行 | 嵌套 sway 作为 mutter 客户端渲染为宿主窗口 |

## 2. 非目标（明确排除）

- **触碰宿主桌面**：Wayland 客户端互盲是协议设计，不做任何绕过
- **持久虚拟键盘**（自研 zwp 协议客户端）：wtype + 250ms settle 已够用；
  scanner 跨接口引用的实现复杂度不值得（原型已验证并回退，代码留档）
- **后端热插拔**：wlroots 后端在进程启动时固定；"headless 世界事后长出
  原生窗口"需自研组合器（Smithay），记录为远期方向
- **CRIU 进程迁移**：GUI 应用的显示连接无法在恢复时重接

## 3. 架构总览（松耦合三道墙）

### 墙 1：工具面零变化

不新增任何 MCP 工具。唯一 API 变化：

```
lgh_display_create 加可选参数 backend:
    "xvfb"（默认，现状）
  | "wayland-headless"（隐形沙盒，sway headless）
  | "wayland-nested"（原生窗口沙盒，sway wayland 后端）
  | "auto"（宿主有 Wayland 会话 → wayland-nested；否则 wayland-headless）
```

`lgh_display_info` / `lgh_list_displays` 返回值增加 `backend` 字段（信息性）。
其余 36 个工具语义不变，由驱动层按 backend 分派。

### 墙 2：代码隔离——新增 crate `lgh-wayland`

所有 Wayland 协议客户端代码进独立 crate，**只依赖 `lgh-core`**（trait 定义）
与外部二进制（sway/wtype/grim/wl-copy 以子进程方式使用，不链接其库）。
X11 代码（x11rb/XTest/AT-SPI）一行不动。

```
lgh-core（trait 定义，不动）
   ↑                    ↑
lgh-driver           lgh-wayland（新 crate）
(X11 实现)            ↑
   ↑                外部二进制：sway / wtype / grim / wl-copy / wayvnc
lgh-runtime（装配点：按 DisplayConfig.backend 分派）
   ↑
lgh-daemon（MCP 层，零改动）
```

### 墙 3：装配点集中在一处

探针证明唯一需要"认得协议"的决策有四个，全部收在 `lgh-runtime`：

| 装配点 | X11（现状） | Wayland 新增 |
|---|---|---|
| 组合器进程 | `XvfbBackend` + `OpenboxWM` | `SwayBackend`（WM 内置，`wm` 字段 = None，已是 Option） |
| InputDriver | XTest + wtype | 虚拟指针（zwlr_virtual_pointer）+ wtype |
| CaptureDriver | X11 GetImage | grim（wlr-screencopy） |
| WindowDriver | X11 + wmctrl | sway IPC（con_id 寻址） |
| A11yDriver | AT-SPI（X11 坐标修正） | AT-SPI **大体复用**（D-Bus 与显示协议无关，仅坐标修正分支） |
| 交互视图 | x11vnc | wayvnc（daemon/vnc.rs 一处分支） |
| 预览帧源 | X11 GetImage | grim（lgh/preview 一处数据源分支） |

## 3.5 统一原则（松耦合的不变量）

工具层**零协议分支**是本设计的核心不变量，配套三条结构性统一：

1. **工具层只认 trait**：`lgh-daemon` 的所有工具 handler 只经由
   `lgh-core` 的驱动 trait（Input/Capture/Window/A11y/Clipboard）分派，
   出现任何 `match backend` 即视为架构违规。以一个编译期/测试期检查固化
   （grep 守卫进入 CI）。
2. **剪贴板升格为 trait**：现有 `runtime/clipboard.rs` 的 hub 逻辑拆为
   `ClipboardDriver` trait——X11 spoke（xclip 经 ：0）与 Wayland spoke
   （wl-copy/wl-paste 经 wlr-data-control）各为一份实现；hub 规则
   （origin 过滤/方向策略）是纯逻辑，与协议无关，整体保留。
3. **预览帧源 trait**：`lgh-preview` 的 cell 经 `FrameSource` trait 取帧
   （X11 GetImage | grim 子进程），面板布局/悬停/分页逻辑与帧源完全解耦。

交互（AI 注入 vs 用户原生输入）同样统一：两种后端下 AI 都走虚拟设备
协议，用户走宿主路由——驱动层内差异，工具层无感知。

### cua 近期 Linux 提交的可借鉴清单（2025-08 之后）

逐条对应到本设计的落点：

| cua 提交 | 教训 | 本设计落点 |
|---|---|---|
| 8c515ca74 AT-SPI walk 按窗口收敛 | AT-SPI 每进程一棵树，多窗口应用全量暴露；必须按请求窗口过滤 | A11yDriver 的树查询加 window 过滤参数（两协议共用） |
| ee77164b2 AT-SPI action 按 name 选择 | 按索引/位置选 action 在 UI 更新后漂移 | `lgh_click_element`/`invoke_menu` 坚持 name/identity 寻址（已有 identity 机制，固化为契约） |
| d86e49ba4 选择正确的 AT-SPI 应用树 | 多应用场景下先选对 application 节点 | A11yDriver 查询前置应用解析 |
| 669c91778 native Wayland 修饰键保持 | 修饰键必须与按键组成 chord 一次性发送，单独丢弃会静默变字符 | Wayland 键盘实现的验收用例（modifiers+key 同批次） |
| c78c1d873 拒绝未证实的 Wayland 窗口捕获 | 无法证明像素归属目标表面时 fail closed | 沙盒内 grim 全屏捕获天然自证（自有组合器）；保留"精确拒绝"契约 |
| d1c5a7a90 应用退出后有界快照 | 应用退出后不得再抓其残留帧 | capture 路径加存活检查（两协议共用） |
| 5448c449c MIT-SHM 加速 X11 捕获 | X11 捕获性能路线 | X11 后端可选优化；Wayland 对应=持久 screencopy 连接（远期） |
| fb4b492cd .desktop 解析 launch_app | 应用名经 .desktop 条目解析 + xdg-open 失败要显式化 | lgh_app_launch 的名称解析（list_apps 已合并 .desktop，补 spawn 路径） |
| f91d78e49 收割 launch_app 子进程 | spawn 后不 wait 即僵尸 | 已实现（收尸线程）✓ |

## 4. 模块设计

### 4.1 `lgh-runtime/src/compositor/`（进程编排，不依赖 wayland-client）

```
compositor/
├── mod.rs          // CompositorBackend trait + 分派
├── xvfb.rs         // 现有 XvfbBackend 移入（行为不变）
└── sway.rs         // 新增：SwayBackend
```

`SwayBackend::start(config)` 职责（探针 world.rs 已验证的实现）：

1. 写配置文件（bar 隐藏 + 输出模式）
2. spawn sway，环境按模式：
   - headless：`WLR_BACKENDS=headless` + 移除 `WAYLAND_DISPLAY`
   - nested：`WLR_BACKENDS=wayland` + 保留 `WAYLAND_DISPLAY`（连接宿主）
   - 共通：`WLR_RENDERER=pixman`（VM/无 GPU 权限时唯一可靠路径）、
     `WLR_LIBINPUT_NO_DEVICES=1`、移除 `DISPLAY`
3. socket 发现：**exec 探测法**——`swaymsg exec 'echo $WAYLAND_DISPLAY
   $DISPLAY > <probe>'`，从 sway 自己的环境读取权威值（比 before/after
   快照差分可靠：后者会被 mutter 动态创建的 socket 污染——探针实测教训）
4. 输出保障：`get_outputs` 为空则 `create_output`，再设 mode
5. IPC socket：`sway-ipc.<uid>.<pid>.sock`（注意含 uid——探针踩坑）
6. 生命周期：SIGTERM-first、收尸线程（复用 ManagedProcess）

### 4.2 `lgh-wayland` crate（协议客户端，实现 lgh-core traits）

```
lgh-wayland/src/
├── lib.rs           // WaylandDriver：实现 Input/Capture/Window/A11yDriver
├── pointer.rs       // zwlr_virtual_pointer_v1 会话（绝对坐标/拖拽保连）
├── keyboard.rs      // wtype 封装（外部二进制）
├── capture.rs       // grim 封装（外部二进制）
└── toplevels.rs     // sway IPC：窗口枚举/激活/关闭
```

- **指针**：zwlr_virtual_pointer_v1（绝对坐标 motion_absolute + button +
  frame）；拖拽 = 单连接内 press→move…→release（保连防丢按键）
- **键盘**：wtype 子进程（外部二进制，keymap 由它管理）
- **捕获**：grim 子进程（wlr-screencopy）
- **窗口**：swaymsg IPC（sway 特有，但沙盒组合器就是 sway——见 §7）
- 已知竞态与缓解：指针/键盘操作后 250ms settle（wtype 新连接的 keymap
  竞态会丢首键——探针实测 "12+34"→"2+34"）

### 4.3 `Display` 集成（lgh-runtime，最小足迹）

- `DisplayConfig` 增加 `backend: Backend` 枚举（X11 默认）
- `Display` 字段：`xserver`/`wm` 已是 Option ✓；`driver` 字段从
  `Arc<DefaultDriver>` 改为按 backend 持有相应驱动实例
- `DisplayKind`（Harness/External）不变——backend 是正交维度

### 4.4 预览面板（lgh/preview，一处数据源分支）

- 面板本体（X11 窗口，XWayland 上）不动——任何 Wayland 宿主都有 XWayland
- 帧源抽象：`FrameSource::X11(display)` | `FrameSource::Grim(socket)`，
  cell 按 display.backend 选择
- 定位：EWMH `_NET_MOVERESIZE_WINDOW`（mutter 忽略普通 ConfigureWindow——
  探针踩坑；OR 窗口 mutter 不渲染，必须用受管窗口）

### 4.5 交互视图（lgh/daemon/vnc.rs，一处分支）

- X11：x11vnc（现状）
- Wayland：wayvnc `-o <output>`（按需 spawn，用后即走）
- 嵌套后端的世界：用户可直接操作原生窗口，VNC 仅为补充通道

### 4.6 剪贴板（第二期，本设计只做接口预留）

- 沙盒端：wl-copy / wl-paste（wlr-data-control，sway 原生支持）
- 宿主 spoke：xclip 经 XWayland 桥（mutter 自动同步 X11↔Wayland 剪贴板）
- hub 规则（origin 过滤/方向策略）从现有 clipboard.rs 1:1 移植

## 5. 工具面变化（回答"是否需要新增工具"）

**不新增任何 MCP 工具。** 唯一变化：`lgh_display_create` 的可选 `backend`
参数（默认 "xvfb"，完全向后兼容）。`lgh_list_displays`/`lgh_display_info`
返回加 `backend` 字段。

## 6. 已知限制（毕业版遗留问题）

| 问题 | 缓解 | 根治方向 |
|---|---|---|
| 键盘首键竞态 | 输入工具尾部 250ms settle | sway 层调试 / 自研组合器 |
| 后端热插拔 | 不支持，创建时选定 | Smithay 自研组合器 |
| mutter 不渲染 OR 窗口 | 面板用受管窗口 + EWMH | — |
| 宿主截图通道偶发黑帧 | 与沙盒无关（mutter/DRM 瞬时故障），重试即恢复 | — |

## 7. 沙盒组合器选型：为什么是 sway

- **多实例并行**：mutter 深度绑定 GNOME 会话（D-Bus 名字/设置栈），无法
  N 个并存；sway 是纯黑盒（spawn/kill 随意）
- **协议齐全**：screencopy、virtual input、foreign-toplevel 全是 wlroots
  标准件；mutter 需要走 portal + libei + Shell 扩展（cua-driver 的泥潭）
- **双后端**：headless/wayland 一字之差（环境变量），对应两种产品形态
- **黑盒化**：spawn + 配置文件 + kill，无会话栈依赖

## 8. 分期与验收标准

### 第一期：wayland-headless

- `lgh-wayland` crate（Input/Capture/WindowDriver）
- `lgh-runtime/compositor/sway.rs`（headless）
- daemon/vnc.rs、lgh/preview 的分支接线
- 验收：现有测试全绿（X11 零回归）；Wayland 冒烟等价探针
  （create → launch → click → type → capture → destroy 全通过）；
  modifiers chord 用例（cua 669c91778 教训）；trait-only 守卫测试

### 第二期：wayland-nested + 剪贴板

- nested 后端（EWMH 面板定位已验证）
- 剪贴板 wayland spoke + hub 规则移植
- 验收：nested 窗口原生交互；剪贴板规则等价 lxh 设计

### 第三期（远期，不在本设计范围）

- Smithay 自研组合器（后端热插拔、持久虚拟键盘）
- 宿主验证矩阵扩展（KWin/deepin/Hyprland）

## 9. 打包

- Ubuntu/deb：已验证（107+17 包解包方案，或正式打包）
- rpm/静态捆绑：工程活，不阻塞设计
- 二进制发现：PATH 优先；预留 `LGH_TOOLCHAIN_DIR` 覆盖（无头宿主）

## 附录 A：原型验证证据（wayland-probe）

- 5 沙盒并行（xterm/calculator/text-editor/nautilus/firefox）全部截图验证
- 启动 0.365s（σ≈2ms）/ 销毁 24–66ms
- 组合器 PSS ~15MB；空闲 CPU ~0.4%/沙盒
- 计算器端到端注入：键盘输入表达式（首键竞态已定位并缓解）
- 嵌套后端：原生窗口出现在宿主桌面（mutter 客户端）
- 已修 bug：grim 管道死锁、mutter 忽略 ConfigureWindow、
  sway IPC 路径含 uid、OR 窗口不渲染、spawn 后僵尸、daemon 单例守卫
