# Wayland 后端集成——详细重构方案（feat/wayland）

> 状态：草案 v2（详细设计），待评审。
> 原型验证：`~/桌面/wayland-probe`（关键假设全部实测，证据见附录 A）。
> 本文替换 v1 草案；§1–3、§5–9 为 v1 修订版，§4 为函数级详细重构方案。

## 1. 背景与目标

X11 沙盒（Xvfb + openbox）之外增加 Wayland 后端，使 AI 智能体可在
Wayland 协议的隔离图形世界中操作应用。原型已实测验证：

| 事实 | 数据 |
|---|---|
| 沙盒组合器可多开、可隐形 | 5 个并行 sway（headless），互不可见 |
| 启动/销毁 | 0.365s / 24–66ms |
| 组合器成本 | PSS ~15MB，空闲 CPU ~0.4% |
| 同一工具面覆盖双协议 | 37 个工具无差别工作 |
| 原生嵌窗 | 嵌套 sway 作为 mutter 客户端渲染为宿主窗口 |
| 对照组 | Xvfb :9 (1280×800) RSS 65.7MB vs sway headless 31.8MB |

## 2. 非目标（明确排除）

- **触碰宿主桌面**：Wayland 客户端互盲是协议设计；宿主触达类工具
  （`lgh_list_user_windows`、`lgh_display_attach`）在 Wayland 宿主上
  返回类型化拒绝（fail closed，cua c78c1d873 同款哲学）
- **持久虚拟键盘**（自研 zwp 协议客户端）：wtype + 250ms settle 已够用；
  scanner 跨接口引用的实现复杂度不值得（原型已验证并回退，代码留档）
- **后端热插拔**：wlroots 后端在进程启动时固定；"headless 世界事后长出
  原生窗口"需自研组合器（Smithay），记录为远期方向
- **CRIU 进程迁移**：GUI 应用的显示连接无法在恢复时重接

## 3. 架构总览（松耦合三道墙 + 诚实边界）

### 3.1 工具签名零变化

37 个 MCP 工具签名全部不变。唯一 API 变化：

```
lgh_display_create 加可选参数 backend:
    "xvfb"（默认，现状）
  | "wayland-headless"（隐形沙盒，sway headless）
  | "wayland-nested"（原生窗口沙盒，sway wayland 后端）
  | "auto"（宿主有 Wayland 会话 → wayland-nested；否则 wayland-headless）
```

`lgh_list_displays` / `lgh_display_info` 返回加 `backend` 字段。

### 3.2 语义差异显式管理（cua 31 处 is_wayland 的教训）

工具**签名**统一，部分工具的**语义**不同——差异收敛到驱动层并进入
能力矩阵（§4.7），不在工具层 if/else：

| 能力 | X11 | Wayland (sway headless) | 策略 |
|---|---|---|---|
| 全屏截图 | ✓ XGetImage | ✓ grim | |
| 窗口截图（平铺，无遮挡） | ✓ | ✓ grim+几何裁剪 | sway 平铺权威 |
| 窗口截图（floating 被遮挡） | ✓ 窗口自有内容 | **精确拒绝**（像素归属不可证） | fail closed |
| 点击/拖拽/滚轮 | ✓ XTest | ✓ virtual pointer | |
| 文本输入 | ✓ wtype | ✓ wtype | 首键竞态已缓解 |
| 修饰键组合 | ✓ | ✓ chord（cua 669c91778 教训） | |
| 窗口枚举/聚焦/关闭 | ✓ X11 | ✓ sway IPC | id 令牌化 |
| 光标位置查询 | ✓ XQueryPointer | △ 驱动自追踪注入状态 | |
| AT-SPI 树/坐标 | ✓ | △ 按组合器实测 | 第一期验证项 |
| 宿主桌面枚举/接管 | ✓（:0 同协议） | **✗ 类型化拒绝** | 宿主触达 |

### 3.3 统一不变量（CI 守卫）

工具层 handler 只经驱动 trait 分派——`lgh-daemon` 出现 `match backend`
即测试失败（grep 守卫进 CI）。协议分叉只允许存在于：
`lgh-runtime/src/compositor/`（进程编排）与 `lgh-wayland`（协议实现）。

## 4. 详细重构方案

### 4.0 现状盘点（精确到符号）

| 符号 | 位置 | 现状 | 改动 |
|---|---|---|---|
| `DisplayConfig` | lgh-runtime/display.rs:16 | width/height/depth | +`backend: Backend` |
| `Display::create(id, display, config)` | display.rs:47 | 硬编码 Xvfb+openbox+DefaultDriver | 分派 X11/Wayland 两路径 |
| `Display.driver -> &DefaultDriver` | display.rs:138 | 具体类型 | `Arc<dyn Driver>` |
| `Display.xserver/wm: Option<ManagedProcess>` | display.rs:36-37 | Xvfb/openbox | Wayland: sway 填 xserver 位，wm=None |
| `Runtime.next_display: AtomicU32` | lib.rs:29 | `:N` X11 显示号分配 | 仅 X11 路径使用 |
| `Runtime.create_display` | lib.rs:53 | 分配 :N + Display::create | Wayland 分支跳过分配 |
| `DaemonState.drivers: Arc<RwLock<HashMap<String, Arc<dyn Driver>>>>` | handlers.rs:24 | **已是 trait 对象** | 零改动 ✓ |
| handlers `display_create` | handlers.rs:85 附近 | `DisplayConfig::default()` | 读 backend 参数 |
| `vnc.rs` x11vnc | daemon/vnc.rs | x11vnc -display :N | +wayvnc 分支 |
| preview cell 帧读取 | preview/cell.rs:435 X11 GetImage | X11 帧源 | +grim 帧源 |
| `ClipboardHub::start_spoke(display)` | runtime/clipboard.rs | X11 spoke | +wayland spoke（二期） |

### 4.1 目标模块树（新增 = +，修改 = ~，不动 = =）

```
lgh/
├── core/       =  Backend 枚举 +；DisplayConfig.backend +
├── driver/     ~  DefaultDriver 不动；Driver trait 语义契约文档化
├── wayland/    +  新 crate：WaylandDriver（协议客户端）
│   ├── Cargo.toml      # deps: lgh-core, tokio, serde_json, sway-ipc 客户端为子进程
│   └── src/
│       ├── lib.rs      # WaylandDriver: impl Driver
│       ├── pointer.rs  # VPtrSession（zwlr_virtual_pointer_v1）
│       ├── keyboard.rs # wtype 封装（子进程，WAYLAND_DISPLAY env）
│       ├── capture.rs  # grim 封装（子进程）
│       ├── toplevels.rs# swaymsg -t get_tree 解析 / con_id 操作
│       └── cursor.rs   # 注入状态自追踪（get_cursor_position）
└── runtime/    ~
    └── compositor/ +   # 新模块（进程编排，不依赖 wayland-client）
        ├── mod.rs      # CompositorHandle 抽象 + 分派
        ├── xvfb.rs     # 现有 XvfbBackend 移入（行为不变）
        └── sway.rs     # SwayBackend（spawn/env/socket 发现/输出/XWayland 探测）
lgh/daemon/   =  零改动（drivers 已是 Arc<dyn Driver>）；
                 仅 handlers display_create 读 backend 参数
lgh/preview/  ~  帧源抽象（X11 GetImage | grim）
```

依赖方向（无环）：`lgh-wayland → lgh-core`；`lgh-runtime → lgh-driver`；
`lgh-runtime/compositor` 只 spawn 外部二进制，无新依赖。

### 4.2 `lgh-core` 变更（第一步，纯加法）

```rust
// Backend 枚举（lgh-core）
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Backend {
    #[default]
    Xvfb,          // 现状：Xvfb + openbox（X11 协议）
    SwayHeadless,  // sway headless（Wayland 协议，隐形）
    SwayNested,    // sway wayland 后端（Wayland 协议，宿主原生窗口）
}

// DisplayConfig 增加字段（Default 保持 Xvfb，向后兼容）
pub struct DisplayConfig {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub backend: Backend,   // 新增
}
```

### 4.3 `lgh-runtime/src/compositor/sway.rs`（组合器编排详规）

`SwayBackend::start(config: &SwayConfig) -> Result<SwayHandle, LghError>`
（从探针 world.rs 移植，已验证逻辑，tokio 化适配 ManagedProcess）：

```
步骤                              验证状态
1. 快照现有 wayland-* socket 集合   ✓（探针 before/after 差分）
2. 写配置文件 w-{id}.conf：
   bar { mode invisible }          ✓
3. spawn sway：
   env = WLR_BACKENDS=headless|wayland
       + WLR_RENDERER=pixman       ✓（VM 无 GPU 权限实测）
       + WLR_LIBINPUT_NO_DEVICES=1
       + 移除 DISPLAY               ✓
       + nested 模式保留 WAYLAND_DISPLAY（连宿主）✓
       + process_group(0) + 收尸线程 ✓（cua f91d78e49 同款教训）
4. 等 IPC socket：glob sway-ipc.<uid>.<pid>.sock
   （含 uid——探针踩坑：路径是 sway-ipc.1000.<pid> 非 <pid>）✓
5. 等 NEW wayland socket（差分）✓
6. get_outputs 为空 → create_output → 再查
   （wayland 后端输出名 = WL-1 非 HEADLESS-1，动态发现）✓
7. 设 mode ✓
8. XWayland 探测：swaymsg exec 'echo $DISPLAY > probe'
   （沙盒内 Xwayland 自动拉起，:N 由 sway 分配）✓
9. 返回 SwayHandle { sway_pid, ipc, wayland_display,
                     x_display, output, width, height }
```

销毁 `SwayBackend::stop(handle)`：SIGTERM-first（sway 自清 socket）→
3s 宽限 → SIGKILL → 清扫 wayland-N/.lock/sway-ipc 文件（SIGKILL 残留，
探针实测 456 个 lock 的教训）。

### 4.4 `lgh-wayland` 驱动详规（类型签名级）

```rust
pub struct WaylandDriver {
    display: String,          // wayland-N socket 名（非 :N）
    ipc: PathBuf,             // sway IPC socket
    output: String,           // 输出名
    width: u32, height: u32,
    cursor: Mutex<(u32, u32)>,// 注入状态自追踪（get_cursor_position 用）
}
impl WaylandDriver {
    pub fn new(world: &SwayHandle) -> Result<Self, LghError>;
}

// trait 实现要点（对照探针已验证代码）：
impl InputDriver for WaylandDriver {
    // click/move/scroll/drag → VPtrSession（zwlr_virtual_pointer_v1）
    //   move_abs: motion_absolute(t, x, y, W, H) + frame   ✓探针
    //   drag: 单连接 press→move…→release（保连防丢按键）    ✓探针
    //   操作后 250ms settle（wtype keymap 竞态，探针实测
    //   "12+34"→"2+34"）                                   ✓已缓解
    // type_text/key → wtype 子进程（WAYLAND_DISPLAY env 注入）✓探针
    // get_cursor_position → 返回自追踪 cursor 状态
    //   （Wayland 无查询协议；注入是唯一写路径，自追踪即权威）
}
impl CaptureDriver for WaylandDriver {
    // screenshot → grim - (stdout) → 解码 → Screenshot          ✓探针
    // screenshot_window → sway IPC 几何 + grim 全屏裁剪：
    //   平铺（无遮挡）→ 精确 ✓
    //   floating 被遮挡 → LghError::Unsupported（精确拒绝，
    //   cua c78c1d873 同款 fail-closed）                     △待验收
}
impl WindowDriver for WaylandDriver {
    // 枚举：swaymsg -t get_tree 递归解析（con_id/pid/app_id/几何）
    // focus/close：swaymsg [con_id=N] focus|kill            ✓探针
    // set_window_frame：swaymsg [con_id=N] move/resize
    //   （平铺布局下为建议值——文档化）
}
// A11yDriver：第一期复用 AtspiA11y（AT-SPI 走 D-Bus，与显示协议无关）；
//   坐标空间差异按 sway 实测修正（探针未验证项，见 §8）
```

### 4.5 `Display` / `Runtime` 集成（签名变更清单）

```rust
// Display 字段与构造（display.rs）
pub struct Display {
    id: String,
    display: String,          // X11=":N"；Wayland="wayland-N"（socket 名）
    kind: DisplayKind,
    backend: Backend,         // 新增
    width: u32, height: u32,
    compositor: Option<ManagedProcess>,  // 改名：xserver → compositor（语义诚实）
    wm: Option<ManagedProcess>,          // Wayland = None
    apps: Mutex<Vec<ManagedProcess>>,
    driver: Arc<dyn Driver>,  // 泛化（handlers 已按 trait 对象使用 ✓）
    owner: Mutex<Option<String>>,
}

// Display::create 分派（X11 路径逻辑原样提取，行为不变）
pub async fn create(id: String, config: DisplayConfig) -> Result<Self, LghError> {
    match config.backend {
        Backend::Xvfb => Self::create_x11(id, config).await,      // 现有逻辑
        Backend::SwayHeadless | Backend::SwayNested =>
            Self::create_wayland(id, config).await,               // 新路径
    }
}

// Runtime.create_display：display 号分配（:N）仅 Xvfb 路径执行；
// Wayland 路径 display 字符串来自 SwayHandle 的探测结果。
```

`launch_app` 的环境注入按 backend：
- X11：`DISPLAY=:N`（现状）
- Wayland：`WAYLAND_DISPLAY=wayland-N` + `DISPLAY=:M`（沙盒内 Xwayland，
  探测值）——与 X11 相反的主次（探针实测 firefox/calculator 正确落位）

### 4.6 周边接线（每处一个分支）

| 位置 | X11 | Wayland |
|---|---|---|
| daemon/vnc.rs | x11vnc -display :N | wayvnc（WAYLAND_DISPLAY env，-o output）|
| lgh/preview 帧源 | X11 GetImage（cell.rs:435 现状） | grim 子进程 |
| runtime/clipboard spoke | X11（现状） | wl-copy/wl-paste（二期） |
| handler `lgh_display_create` | `DisplayConfig::default()` | 读 backend 参数构造 config |

### 4.7 实施序列（每步编译 + 测试绿后才进下一步）

| 步 | 内容 | 门 |
|---|---|---|
| S1 | lgh-core：Backend 枚举 + DisplayConfig.backend（默认 Xvfb） | 编译绿；现有测试零改动通过 |
| S2 | Display::create 分派重构（X11 逻辑原样提取 create_x11） | X11 回归测试全绿 |
| S3 | compositor/sway.rs：SwayBackend（headless）+ 单测（socket 发现/输出发现纯函数） | 编译绿 + 单测绿 |
| S4 | 集成冒烟：open(wayland-headless) → spawn xterm → capture → seal | 探针等价冒烟通过 |
| S5 | lgh-wayland crate：WaylandDriver（Input/Capture/Window）+ Display.driver 泛化 | X11 回归绿 + Wayland 冒烟（click/type/capture） |
| S6 | daemon/vnc.rs wayvnc 分支 + preview 帧源 | 面板缩略图 + enter 交互实测 |
| S7 | nested 后端（env 分支 + EWMH 已验证） | 原生窗口出现 + 原生输入实测 |
| S8 | 文档/CI 守卫（trait-only grep）/ capability matrix 入库 | CI 绿 |

回滚：每步一个 commit，任何一步失败 `git reset` 回上一步。

### 4.8 能力矩阵（验收表的骨架，逐格对应测试用例）

| 能力 | X11 (Xvfb) | Wayland (sway headless) | 备注 |
|---|---|---|---|
| 全屏截图 | ✓ XGetImage | ✓ grim | |
| 窗口截图（平铺） | ✓ | ✓ grim+几何 | |
| 窗口截图（floating 被遮挡） | ✓ | ✗ 精确拒绝 | 像素归属 |
| 点击/拖拽/滚轮 | ✓ XTest | ✓ virtual pointer | |
| 文本输入 | ✓ wtype | ✓ wtype | 首键竞态已缓解 |
| 修饰键组合 | ✓ | ✓ chord | cua 669c91778 |
| 窗口枚举/聚焦/关闭 | ✓ X11 | ✓ sway IPC | |
| AT-SPI 树 | ✓ | △ 第一期实测 | 坐标空间 |
| 宿主桌面枚举/接管 | ✓ | ✗ 类型化拒绝 | 宿主触达 |

## 5. 工具面变化（回答"是否需要新增工具"）

**不新增任何 MCP 工具。** 唯一 API 变化：`lgh_display_create` 的可选
`backend` 参数（默认 "xvfb"，完全向后兼容）。`lgh_list_displays`/
`lgh_display_info` 返回加 `backend` 字段。剪贴板工具已存在——wayland
spoke 接 wl-clipboard，工具面不变。

## 6. 已知限制（毕业版遗留问题）

| 问题 | 缓解 | 根治方向 |
|---|---|---|
| 键盘首键竞态 | 输入工具尾部 250ms settle | sway 层调试 / Smithay 自研组合器 |
| 后端热插拔 | 不支持，创建时选定 | Smithay 自研组合器 |
| mutter 不渲染 OR 窗口 | 面板用受管窗口 + EWMH | — |
| 宿主截图通道偶发黑帧 | 与沙盒无关（mutter/DRM 瞬时故障），重试即恢复 | — |
| floating 窗口被遮挡时窗口截图拒绝 | 精确拒绝契约 | 平铺布局天然规避 |

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
