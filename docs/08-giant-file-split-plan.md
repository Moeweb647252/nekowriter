# 08 巨型文件拆分计划（只移动、不改行为）

本文件是 `docs/02` §2.4 第 2 条（文件规模）的执行方案，也是 `docs/05` F-09 与
`docs/06` F-09「有意延期」那一项的接续。那一轮延期的理由今天仍然成立：纯搬迁的万行级改动
不能与行为修复混在同一批文件里，否则「哪一处改动改变了行为」无法核对。因此本轮只做搬迁，
单独成轮、每批一次提交、每批单独可回滚。

**范围**（三个文件）：

| 文件 | 行数 | 生产代码 | 测试代码 | 备注 |
| --- | ---: | ---: | ---: | --- |
| `crates/nekowriter-store/src/preparation.rs` | 13,205 | 13,205 | 0 | 47 个 `impl Store` 方法 + 约 150 个自由函数；**自身没有测试模块** |
| `crates/nekowriter-store/src/lib.rs` | 7,443 | 1,437 | 6,005 | `#[cfg(test)] mod tests`（1,438 行起）占了 81% |
| `crates/nekowriter-app/src/main.rs` | 4,240 | 3,081 | 1,159 | 测试模块（3,082 行起） |

**不在范围**（各有独立的一轮，理由见 §8）：`app/scheduler.rs`（按 lane 拆）、`core/lib.rs`、
`store/knowledge.rs`，以及 `ApiError::into_response` 从「一个 466 行的 match」进一步改成
「按错误域分派到若干小函数」——那是行为等价的重写，不是搬迁。

---

## 1. 定义：「只移动、不改行为」的两个不变量

搬迁必须同时满足两条可机械核对的不变量。

**I1 名称解析不变。** 任何被移动的项，在它原来的路径上仍然可见：`lib.rs` 里的
`pub use preparation::{...}`、15 个 `store::*` 兄弟模块里的 `use super::{db_id, ...}`、
10 个文件里的 `super::insert_app_event(...)`，一行都不改。做法是在原位补一行
`pub(crate) use <新模块>::{...};`（或 `pub use`，按原可见性）。因此每个批次的 diff 必须
**只出现在被拆的文件与它新建的模块文件里**；任何调用方文件被改动，这批就不是搬迁。

**I2 正文不变。** 归一化（丢缩进、空行、整条 `use` 语句、`pub` / `pub(crate)` 前缀与函数签名
的折行）之后，搬迁前后的**定义项逐项有序内容完全相同**（§3 的脚本；2026-09-16 由「行池化多重集」
改成按定义项比对，理由与探针证据见 §9 最后一条记录）。可见性从私有放宽到 `pub(crate)` 是允许的
编辑（新子模块之间要互相看见），它正好被归一化覆盖；除此之外任何一行变化都按行为改动来审，
**包括项内调换两行**。

每个批次允许的连带改动只有三类，且必须在提交信息里逐条点名：

1. 新文件与被拆文件里的 `mod` / `use` / `pub(crate) use` 行；
2. 源码形状测试的 `include_str!` / 目录表（见 §4 第 1 条）；
3. 模块文档注释在文件之间的搬迁。

不允许：函数体任何一行、签名、字面量、错误码与文案、事件类型、`#[allow]`、`#[cfg]` 的移动。

---

## 2. 基线（2026-09-15，`main`）

安全网（`rg -c '^\s*#\[(tokio::)?test\]'` 的当前计数）：

| crate | 测试数 | 明确覆盖本次搬迁目标的部分 |
| --- | ---: | --- |
| `nekowriter-store` | 96 | `lib.rs` 的 46 个：项目/稿本、准备提交、审核、正文工具、协议计数、schema 形状 |
| `nekowriter-app` | 69 | 单元 67（`main.rs` 的 25 个走真实端口 HTTP、`scheduler.rs` 的 30 个驱动 store 命令、其余 12）+ 集成 2 |
| `nekowriter-agent` | 44 | — |
| `nekowriter-core` | 38 | — |
| `xtask` | 4 | `check-move-only` 落地后为 12（§9） |
| `nekowriter-prompt-lab` | 2 | — |
| 合计 | 253 | `cargo test --workspace` |

`preparation.rs` 自身 0 个测试，正文工具体、审核委托、引用校验这些路径全部由上面 store 与
app 的测试间接覆盖——这是 Phase 3 必须放在最后、且每批都要跑 `-p nekowriter-store -p
nekowriter-app` 两个 crate 测试的原因。

---

## 3. 每批都跑的校验协议

### V0 阶段基线（每个阶段开始时一次，记进本文件的「§9 执行记录」）

```bash
cargo fmt --all -- --check
# clippy 只为真正重编译的 crate 打印诊断：要数基线就先强制全量重编译。
find crates -name '*.rs' -exec touch {} +
cargo clippy --workspace --all-targets --message-format short 2>&1 | rg ': (warning|error)' | sort > /tmp/clippy-base.txt
cargo test --workspace
cargo test --workspace -- --list 2>/dev/null | rg ': test$' | sort > /tmp/tests-base.txt
```

`clippy-base.txt` 与 `tests-base.txt` 是后面每一批拿来 diff 的两个基线文件。
**2026-09-15 实测**：`cargo clippy --workspace --all-targets` 是 **0 warning / 0 error**
（`docs/05` F-09 记的 244 条 warning 行已经在它之后的 lint 清理轮里归零），`too_many_lines`
等 `#[allow]` 仍在 `Cargo.toml` 里，本轮不动；`cargo test --workspace` 是 **253 通过 / 0 失败**。

### V1 每批（四道，全部通过才算这批完成）

**第一道：范围。** 改动路径必须落在白名单里。

```bash
git diff --name-only "$BASE"   # BASE 是本批之前的提交
```

只允许出现：被拆的文件、本批新建的模块文件，以及该批次提交信息里点名的形状测试文件。

**第二道：正文（按定义项逐项比对）。** 一次搬迁用一条命令核对（工具与它自己的用例在
`crates/xtask`，P0.1 落地，见 §9）：

```bash
cargo xtask check-move-only --base <基准提交> \
  --from <搬迁前的文件> \
  --into <搬迁后的文件> --into <搬迁后的文件> ...
```

`--from` / `--into` / `--allow` / `--scope` 每个值各一个参数，要多个就把标志重复写（工具
的 `用法：` 里的 `...` 指的是标志可重复，不是「后面跟一串值」——2026-09-16 在本节之外的工作
里踩过一次）。默认允许改动的集合是 `--from ∪ --into`，默认作用域是声明文件所在的
`crates/<名字>`——共享检出里别的 crate 的并行改动不参与判定，要更宽的判定用 `--scope`
（例如 store 的搬迁要带上 `--scope crates`）。`--head <提交>` 可以对着另一个提交核对，省略即
对着工作区。`--list-items` 只把两侧切出来的定义项与项外自由行列出来（键 + 行数），不做判定
——判据按项分组，要人工核对就得能看见这些名字。

**未提交的连续批次**（本仓这个会话的实际状态，见 §7 的假设）不能靠 `--base <提交>` 隔离
单批：`--base HEAD` 会把上一批的搬迁也算进本批。这时用批次状态：上一批结束时
`--write-state <目录>` 把当时 `crates/` 下的 Rust 源码写成快照（P1.2 之后 82 个文件，P1.3
之后 83 个），本批用
`--base-state <目录>` 读基准内容、并对着快照算「哪些文件变了」。`--head-state <目录>`
（2026-09-16 加）是它的对称项：两侧都是快照时，一批的判定不再依赖「工作区恰好停在那批之后」，
因此**任意一批随时可以重跑**。快照是执行者的临时状态，不进仓库（本会话在
`/tmp/nw-split-state/<批次>`）。

两条判据就是 §1 的 I1 与 I2：

- I1 数「改动、新增、删除的文件是否全部落在允许集合里」；
- I2 先把两侧源码切成**定义项**（`fn` / `struct` / `enum` / `trait` / `union` / `const` /
  `static` / `type`），再按 key 分组**逐项比对有序内容**。key 由容器身份与归一化签名组成，
  因此「同一个项换到哪个文件」不影响身份（那正是搬迁要做的），而「两个 `impl` 里的同名方法
  对调实现」会分成两个不同的 key。归一化丢掉缩进、空行、**整条 `use` 语句**（rustfmt 按名字
  长度折行，只看行首会把折行位置当成代码差异）、`pub` / `pub(crate)` / `pub(super)` 前缀；
  **函数签名的折行**也合并成一行（P1.8 加：一个长签名在加了可见性前缀后被 rustfmt 折成四行，
  合并后两侧相等，而参数名 / 类型 / 返回值改了仍然报残留）。`impl` 与 `mod` 是**透明容器**：
  头行不参与比对（`impl Store` 可以合法地一分为二、`#[cfg(test)] mod tests` 可以合法地从一个
  变成多个），但 `impl` 头进内部每一项的 key，`mod` 的名字不进（搬测试时它会在每个目的文件里
  重建）。容器与项上的 **cfg 条件参与比对**：`#[cfg(test)]` 算接线（搬测试必然要在新位置补上
  它），`#[cfg(all(test, feature = "x"))]` 会留下 `cfg: all(feature="x")` 行。**新增的注释行**
  单独报告并放行（新模块要有自己的文档头），**被删掉的注释行**仍算残留。

  于是这三类旧判据看不见的改动现在会被点名到项：**项内调换两行**（`~ 行序改变`）、**把一个项
  拆到两个文件**、**两个 `impl` 里的同名方法对调实现**；而「项整体换文件」「`impl` 一分为二」
  仍照旧放行。探针矩阵（16 条）与十批回验见 §9 最后一条记录。

**口径变更（2026-09-16）**：旧判据把所有声明文件的行**池化**成一个多重集再比，于是行/函数体的
置换、跨文件再分配都完全隐形；新判据按项比。工具输出随之从「归一化代码行 X → Y，残留 N」改成
「定义项 …，项内 … 行，项外自由行 …；残留 N」。**§9 里各批记录的「归一化代码行 X → Y」是旧口径
的数字，与新数字不可互比**（旧数字作为「当时那版判据算出的结论」仍然成立，结论已在新判据下重跑
复核）。

**三句用法上的硬话（P1.6 撞出来的）**（P1.7、P1.8 各又撞上一次；第三句撞出在 2026-09-16 的复核里）：① `--from` 决定基准集合，要把这一批**包含被搬代码的
基准文件**全部列出——包括早前批次已搬出去、这一批又要改的文件（P1.6 的 `http/mod.rs`），
否则它们**既有**的代码会全被算成本批残留（第一次跑就多报 260 行）。② 归一化按行比对，
**改写一条既有注释等于删一条**：新模块的说明要追加成新行，不要改写旧行（P1.6 在三行旧注释上
撞到过，残留 3 行；P1.7 又把两句模块文档改写成「P1.6 …；P1.7 …」，同样报了 2 行残留；
两次都是旧行原样保留 + 追写一行后归零）。③ **检查器自身的源码不属于「被搬的代码」**：修判据
的那几批里 `crates/xtask/src/main.rs` 也在改，但它只进 `--allow`（I1 看得见），不进
`--from` / `--into`——否则工具自己的改动会被当成「这批搬错了」（P1.9 → P1.10 实测会多报 20 行：
P1.10 新增的 `fn brace_delta` 与 `fn cfg_attributes_are_only_wiring_when_they_wrap_a_module`）。

例（P1.1，`ApiError` 从 `main.rs` 搬到 `http/error.rs`，原处只剩 `mod http;` 与
`pub(crate) use`）：

```bash
cargo xtask check-move-only --base HEAD \
  --from crates/nekowriter-app/src/main.rs \
  --into crates/nekowriter-app/src/main.rs \
  --into crates/nekowriter-app/src/http/mod.rs \
  --into crates/nekowriter-app/src/http/error.rs
```

实测结论见 §9：真实搬迁报 `MOVE-ONLY OK`（归一化 3,890 → 3,903 行、残留 0、放行新增注释
13 行），故意改一行的副本报 `RESIDUAL DIFF` 并逐行点名。

**第三道：编译与 lint 增量。** 移动之后遗留的 `use`、变私有导致的 `dead_code`、少导出一个
名字——都由 clippy 的 warning 列表暴露，这比读 diff 可靠。

```bash
cargo clippy -p <crate> --all-targets --message-format short 2>&1 | sort > /tmp/clippy-now.txt
diff /tmp/clippy-base.txt /tmp/clippy-now.txt   # 期望：没有新增行
```

出现新增行就是这批的真实缺陷（含路径变化带来的重复：同一 warning 换了文件路径是允许的，
签名变化不是）。crate 级的基线用同一份 workspace 基线过滤即可。

**第四道：测试。**

```bash
cargo test -p <crate>
cargo test -p <crate> -- --list 2>/dev/null | rg ': test$' | sort > /tmp/tests-now.txt
diff /tmp/tests-base.txt /tmp/tests-now.txt   # 只允许路径前缀变化
```

- Phase 1 的 P1.1–P1.8：`cargo test -p nekowriter-app` 必须仍是 69 个通过。
- Phase 2、Phase 3 的每一批：`cargo test -p nekowriter-store -p nekowriter-app` 两个 crate
  一起跑（preparation 的路径只有 app 侧的 scheduler 测试走得更深）。
- `--list` 的差异只允许「模块路径前缀」不同；**叶子函数名不同就是测试被改了**，反过来
  leaves 相同而 module path 变化正好是搬迁的预期形状。

**跨缩进层搬迁的例外（P2.3 实测，先看这一条再判 P2.3）**：判据把行归一化后按有序序列比对，
而**折行的位置本身算内容**。把代码从嵌套层（`mod tests` 里面）搬到文件顶层时，后者缩进是 0
（rustfmt 对 out-of-line 模块文件一律按列 0 格式化），行宽预算变大，rustfmt 就会把原本折行的
调用链合并——此时 I2 报的那些行**它报得对**，文本确实变了。这类批次不能声称 MOVE-ONLY OK，
要另给「逐项 token 级比对」（去空白 + 去紧邻 `)`/`]`/`}` 的尾逗号后逐字节相同）把「只是排版」
证掉。同层级文件之间搬（Phase 1 / Phase 2 生产代码的每一批）不会触发这个现象。

### V2 阶段收尾

```bash
cargo fmt --all -- --check && git diff --check
cargo clippy --workspace --all-targets --message-format short 2>&1 | sort | diff /tmp/clippy-base.txt -
cargo test --workspace
```

外加一条人工核对：把 `docs/02` §2.1 的模块表与本阶段的文件系统对账一次（F-08 的教训：
文档写「应该在哪」而没写「已经在哪」）。

---

## 4. 已知地雷（每一条都在这三个文件里实际存在）

1. **三个源码形状测试会跟着搬迁一起坏**，必须在同一批里改，且它们都是「读源码」而不是
   「跑行为」的测试：
   - `store/lib.rs::tests::transaction_external_recheck_has_a_single_owner`（`lib.rs:2207`）：
     `include_str!("preparation.rs")` 等 9 个硬编码路径 + `functions(source)` 解析器（按
     4 空格缩进切函数体）。**`include_str!` 是编译期**——`preparation.rs` 变成目录的那一刻
     这批就编不过。解析器本身对搬迁是安全的：函数搬进 `impl Store` 的子模块后缩进仍是 4 格。
     这个测试在 P2.3 会搬到 `src/tests/` 下，路径要改成 `../lib.rs` 这样的相对路径；它现在
     用 `split("#[cfg(test)]")` 截掉 `lib.rs` 的测试段，测试搬走后这个截断变成空操作——不报错，
     但要在同一批里顺手改成读 `src/tests/` 之外的文件集合。
   - `store/event_types.rs` 的两个测试：递归扫 `src/` 目录下所有 `.rs`——**对拆目录天然安全**，
     但每批之后它都在替我们验证「事件类型字面量仍是同一集合」，属于免费护栏。
   - `app/main.rs::tests::restored_ui_capabilities_are_mounted_in_the_app_shell`：只读
     `webui/src`，与本次搬迁无关，但它在 P1.9 会跟着测试一起搬家，注意 `env!("CARGO_MANIFEST_DIR")`
     的相对路径仍按 crate 根解析，不受影响。
2. **`impl Store` 可以写在子模块里**：Rust 允许同一 crate 内、不同模块中的多个 inherent
   `impl` 块；私有字段的可见性规则是「定义模块及其后代」，而 `Store` 定义在 crate 根，因此
   `preparation/*.rs`、`store/projects.rs` 都能继续直接读 `self.database` / `self.event_notifier`。
   这一条是整份计划成立的前提，P0.1 用最小样例确认一次（搬一个方法出去，跑一次 `cargo check`）。
3. **私有项跨模块要 `pub(crate)`**：DTO、自由函数、常量今天在 `main.rs` / `preparation.rs`
   里是私有的，搬到子模块后必须 `pub(crate)`，否则子模块之间看不见。这是允许的编辑（§1），
   但要注意它同时改变的是「谁能看见」，所以每个批次结束都要看一眼 `cargo public-api` 之类的
   外部可见面是否变化——本仓的对外面只有 `nekowriter_store::*` 的 `pub use` 清单，正常情况下
   一行不动。
4. **行号会漂移**：本文件的每张表都写「当前行号参考」，执行时一律用函数名/类型名定位
   （`rg -n "fn <名字>"`），不要用行号 `sed`。
5. **`Cargo.toml` 的 lint 注释点名了这两个文件**（「`preparation.rs` 与 `lib.rs` 里的正文/审核
   命令超过 100 行」）：它是文档，不是行为，留到 Phase 4 收口时改。
6. **不要在一次提交里合并两个不相邻的区块**：合并之后 clippy/测试一旦出现新信号就无法归因。
   相邻且合计 ≤ 1,200 行的两条可以合并，其余默认不合。

---

## 5. 分批计划

每行 = 一个提交 = 一次 V1。行号是写作时的事实，执行时按名字定位。

### Phase 0 基线与护栏（不改代码）

| 批次 | 内容 | 校验 |
| --- | --- | --- |
| P0.1 | 用最小样例确认 §4 第 2 条（`impl Store` 可跨模块）与 §3 第二道脚本对真实搬迁有效；把脚本与实测结果记进本文件 | 两次 `cargo check -p nekowriter-store` + 一次故意改行的负例 |
| P0.2 | 跑 V0，把 clippy 与测试基线写进本文件「阶段记录」 | V0 四条命令 |
| P0.3 | 开分支 `refactor/giant-file-split`；每阶段末打 tag（`split-phase1` …）作为整阶段的对照点 | `git status` 干净 |

### Phase 1 `app/main.rs` → `app/src/http/`（净安全网最强：69 个测试，含真实端口 HTTP）

目标形状：`main.rs` 只剩 imports、`mod` 声明与 `fn main()`（约 80 行）；HTTP 相关的全部代码
在 `http/` 下；`router()` 移到 `http/mod.rs`（路由表属于 HTTP 层），`main.rs` 用
`use http::router;` 让调用点字节不变。各行预估行数按当前区间长度给出，合计略大于总行数
（`Cli`、`StartupError`、`AppState` 几处被相邻批次交叉引用），误差在 ±10%。

| 批次 | 移动内容（名字） | 目标 | 当前行参考 | 预估行 | 本批额外校验 |
| --- | --- | --- | --- | ---: | --- |
| P1.1 | `ErrorEnvelope` `ErrorBody` `ApiError` + 12 个 `From` impl + `Display` + `IntoResponse::into_response`（466 行 match） | `http/error.rs` | 471–1063 | ~600 | `conflict_errors_keep_stable_code_details_and_request_id`、`invalid_review_decision_is_a_client_validation_error`、`request_limit_errors_keep_decimal_details` |
| P1.2 | 全部 `*Request` / `*Response` DTO、`EventsQuery` `HealthResponse` `EventCursorResponse` `ExportQuery` | `http/dto.rs` | 163–482、1937–1958、2804–2833 | ~380 | `review_decision_accepts_the_payload_the_ui_sends`、`answer_question_request_accepts_the_payload_the_ui_sends`、`embedding_requests_accept_the_payload_the_ui_sends` |
| P1.3 | `Cli` `StartupError` `StartupStep` + impl、`startup_failure*`、`Startup`、`prepare`、`ProcessLock` + impl、`default_data_dir` `resolve_data_dir` `default_web_dir` | `startup.rs` | 85–162、1071–1375、3058–3081 | ~410 | `startup_failure_messages_name_the_directory_step_cause_and_way_forward`、`default_web_dir_lives_next_to_the_executable`、`crates/nekowriter-app/tests/startup_failure.rs` |
| P1.4 | `AppState` `EventStreamState`（handler 与 SSE 的共同输入） | `http/state.rs` | 100–139 | ~40 | `cargo test -p nekowriter-app` 全绿即可 |
| P1.5 | `router` `api_not_found` `shutdown_service` → `http/mod.rs`；`require_trusted_write` `ensure_trusted_write_request` `is_loopback_host` `allowed_write_origins` + `VITE_ORIGINS` → `http/gate.rs`；`serve_client_route` `is_static_asset_path` `serve_index_html` → `http/static_files.rs` | 三个文件都对得上 | 2026-09-16 执行（§9） | 实测 370 行 | `write_trust_is_enforced_by_the_router_not_by_handlers`、`deep_links_fall_back_to_the_entry_html_without_swallowing_api_errors`、`static_dir_serves_client_routes_and_reports_missing_assets_honestly` |
| P1.6 | 系统与设置 handler：`health` `get_capabilities` `get_system_settings` `save_system_settings` `system_settings_response` `test_model_connection` `persist_connection_test`；提示词覆盖与预览 handler | `http/handlers/system.rs`、`http/handlers/prompts.rs` | 171–391、1274–1405 | ~350 | `system_model_credential_can_be_cleared_over_http`、`embedding_settings_response_matches_the_interface_shape`、`embedding_failures_name_the_concrete_reason` |
| P1.7 | 项目、版本与导出 handler：`list_projects` `get_project` `list_versions` `get_version_snapshot` `export_version` `content_disposition` `create_project` `create_version` `select_version` `copy_version` `get_project_settings` `save_project_settings` | `http/handlers/projects.rs` | 392–419、432–472、1214–1273、1406–1437 | ~160 | `state_revision_requests_require_decimal_strings` |
| P1.8 | 章节与正文 handler：`list_chapters` `advance_to_next_chapter` … `submit_chapter_final_review`；准备 handler：`get_preparation` `save_novel_brief` `start_preparation` `submit_preparation_artifacts` `decide_review` `delegate_review` `cancel_review_delegation` | `http/handlers/chapters.rs`、`http/handlers/preparation.rs` | 473–823、962–1016、1039–1173 | ~500 | 这两个文件各拆一次提交（先章节、后准备）；`ensure_model_ready` 与 `enqueue_queued_writing_run` 进 `http/handlers/mod.rs` |
| P1.9 | 事件面：`get_event_cursor` `stream_events` `next_sse_event` `to_sse_event` `to_preview_delta_sse_event` `event_cursor` `stream_reset_required` `stream_reset_event` | `http/handlers/events.rs` | 420–431、824–961 | ~150 | `event_stream_requires_a_snapshot_reset_outside_its_retained_window`、`preview_delta_shares_sse_without_a_persistent_id`、`event_cursor_endpoint_reports_the_retained_global_watermark` |
| P1.10 | `main.rs` 内 25 个测试按主题搬进对应模块的 `#[cfg(test)] mod tests`（启动类进 `startup.rs`、SSE 进 `events.rs`、写信任进 `gate.rs`…），共享 harness（`accumulation_app` `spawn_app` `spawn_app_with_web_dir` `new_accumulation_space` `content_type`）留在 `http/tests.rs` | 分散 | 3082–4240 | ~1,160 | leaves 集合必须与基线一致 |

### Phase 1 的 handler 模块边界（P1.5 定稿；P1.6–P1.10 不再重新决定）

P1.6–P1.10 的边界一次定死：位置、命名、每个函数归哪个文件、每个文件配哪些测试。以后每批开头
不再讨论划分；上表与本节冲突时以本节为准。

**规则**

1. 位置与命名：`crates/nekowriter-app/src/http/handlers/<域>.rs`，一个 API 域一个文件；域名与
   路由前缀、前端 `webui/src/lib/api.ts` 的域分组同名，于是路由表里的一行就能对上一个文件。
   批次 = 文件（P1.6 与 P1.8 各两个文件，每个文件一次提交，回滚粒度见 §7）。
2. 跨域共用的 handler 入口放 `http/handlers/mod.rs`（它同时声明子模块）：目前只有两个，
   `ensure_model_ready`（能力预检）与 `enqueue_queued_writing_run`（把排队的写作重新入队），
   它们被章节域与准备域的 handler 共用（调用点见下表），放在任一域文件里都会让另一域反向依赖。
   其余函数一律进它所属的域文件，包括只被本域 handler 调用的私有 helper。
3. 测试：每个模块自带 `#[cfg(test)] mod tests`；共享 harness 与跨域测试进 `http/tests.rs`
   （P1.10）。`tests.rs` 只放 harness 与跨域用例，不放任何生产逻辑。
4. 不进这批范围：`fn main()` 留在 crate 根（二进制入口的硬约束）；`router`、`api_not_found`、
   `shutdown_service` 已在 `http/mod.rs`（P1.5）；crate 根本来就有域模块（`dashboard.rs`、
   `knowledge.rs`、`manuscript.rs`、`recycle.rs`、`templates.rs`、`prompts.rs`、`operations.rs`、
   `model_settings.rs`、`lifecycle.rs`、`scheduler.rs`）位置不动——Phase 1 只处理 `main.rs`
   自己的代码；把根上的域模块也挪进 `http/` 是另一个决定，不在本计划里。
5. 只允许在接线处变化：handler 搬走后，把 `http/mod.rs` 里那条 `use crate::{...}` 改成
   `use crate::http::handlers::<域>::{...}`——`use` 行属于接线，每个 `.route(...)` 与最后几层
   `.layer(...)` 必须字面不变。P1.5 为此引入的那条 57 个名字的 `use` 会逐批缩到零。

**映射**（行号按 P1.6 后的现行 `main.rs`；函数名是准的）

| 批次 | 文件 | 函数 | 行区间 | 备注 |
| --- | --- | --- | --- | --- |
| P1.6 | `http/handlers/system.rs` | `health` `get_capabilities` `get_system_settings` `save_system_settings` `system_settings_response` `test_model_connection` `persist_connection_test` | 171–391 | `test_model_connection` / `persist_connection_test` 是设置域的一条完整链路（P1.6 已执行，246 行） |
| P1.6 | `http/handlers/prompts.rs` | `get_system_prompts` `get_project_prompts` `save_prompt_override_command` `reset_prompt_override_command` `save_system_prompt_override` `reset_system_prompt_override` `save_project_prompt_override` `reset_project_prompt_override` `preview_prompt` | P1.6 已执行（156 行） | 后四个是前两个 handler 的私有 helper |
| P1.7 | `http/handlers/projects.rs` | `list_projects` `get_project` `list_versions` `get_version_snapshot` `export_version` `content_disposition` `create_project` `create_version` `select_version` `copy_version` `get_project_settings` `save_project_settings` | **三段** 155–181、192–235、977–1068（P1.6 快照行号；P1.7 已执行，193 行） | 非连续：中间隔着事件面与 P1.8 的口径，所以表里的「四段」实际是**三段**（1037 那一段与前一段相邻，且同属项目域，不分开）。省去的 183–191、236–976 是别的域的 handler。每段的边界规则：以项的文档注释/首个属性开头、以闭合大括号加其后那个空行为结尾 |
| P1.8a | `http/handlers/chapters.rs` | `list_chapters` `advance_to_next_chapter` `confirm_novel_completion` `add_extra_chapter` `get_chapter_guidance` `save_chapter_guidance` `start_chapter_outline` `get_chapter_outline` `submit_chapter_outline` `get_chapter_body` `get_document_revision` `append_chapter_body` `replace_chapter_body` `decide_chapter_body_proposal` `submit_run_feedback` `answer_question` `continue_writing_run` `stop_writing_run` `request_chapter_body_pause` `resume_chapter_body_pause` `set_approval_mode` `submit_chapter_final_review` | 158–507（P1.7 快照行号；P1.8 已执行，381 行） | 22 个 handler，本阶段最大的一批；本批与 P1.8b 同一次执行（两个新文件、一次提交） |
| P1.8b | `http/handlers/preparation.rs` | `get_preparation` `save_novel_brief` `start_preparation` `submit_preparation_artifacts` `decide_review` `delegate_review` `cancel_review_delegation` | 647–695、724–857（P1.7 快照行号；P1.8 已执行，211 行） | 与 P1.8a 同一次执行；同批把 `ensure_model_ready`（697–723）与 `enqueue_queued_writing_run`（859–897）放进 `http/handlers/mod.rs`（共 90 行） |
| P1.9 | `http/handlers/events.rs` | `get_event_cursor` `stream_events` `next_sse_event` `to_sse_event` `to_preview_delta_sse_event` `event_cursor` `stream_reset_required` `stream_reset_event` | 129–275（P1.8 快照行号，含 `get_event_cursor` 的文档行；P1.9 已执行，182 行） | 与上表的差异：`get_event_cursor` 从 P1.7 改归这里——`/events` 前缀整块归 `events.rs`，它和 SSE 共用 `EventStreamBounds` 与游标解析 helper；`event_cursor` 与 `stream_reset_required` 因 `main.rs` 里尚未搬走的测试而暂时 `pub(crate)`，P1.10 后应退回私有 |
| P1.10 | `http/tests.rs` + 各模块 `#[cfg(test)] mod tests` | 25 个测试 + harness（`accumulation_app` `spawn_app` `spawn_app_with_web_dir`） | 1069–2233 | 已执行（见 §9）：leaf 名字必须与基线一致（§3 第四道）；归属见下表 |

**P1.10 的测试归属**

| 测试 | 去 |
| --- | --- |
| `startup_failure_messages_name_the_directory_step_cause_and_way_forward`、`default_web_dir_lives_next_to_the_executable` | `startup.rs` |
| `loopback_host_validation_rejects_external_hosts`、`write_trust_is_enforced_by_the_router_not_by_handlers` | `http/gate.rs` |
| `index_html_is_revalidated_before_loading_route_chunks`、`static_dir_serves_client_routes_and_reports_missing_assets_honestly`、`deep_links_fall_back_to_the_entry_html_without_swallowing_api_errors` | `http/static_files.rs` |
| `event_cursor_prefers_last_event_id_and_rejects_invalid_values`、`event_cursor_endpoint_reports_the_retained_global_watermark`、`event_stream_requires_a_snapshot_reset_outside_its_retained_window`、`preview_delta_shares_sse_without_a_persistent_id` | `http/handlers/events.rs` |
| `state_revision_requests_require_decimal_strings`、`review_decision_accepts_the_payload_the_ui_sends`、`answer_question_request_accepts_the_payload_the_ui_sends`、`embedding_requests_accept_the_payload_the_ui_sends` | `http/dto.rs`（形状契约） |
| `system_model_credential_can_be_cleared_over_http`、`embedding_settings_response_matches_the_interface_shape`、`embedding_failures_name_the_concrete_reason` | `http/handlers/system.rs`（两条 embedding 也走设置面） |
| `accumulation_command_writes_candidates_and_replays_without_a_second_engine_call`、`accumulation_engine_failure_writes_nothing`、`accumulation_without_a_production_engine_reports_not_available_and_writes_nothing`、`restored_ui_capabilities_are_mounted_in_the_app_shell` | `http/tests.rs`（跨域：`knowledge` + harness + 真实端口） |
| `conflict_errors_keep_stable_code_details_and_request_id`、`invalid_review_decision_is_a_client_validation_error`、`request_limit_errors_keep_decimal_details` | `http/error.rs` |

（2 + 2 + 3 + 4 + 4 + 3 + 4 + 3 = 25，与基线测试数一致。）

**执行进度**：P1.6 已于 2026-09-16 执行（见 §9），两个文件的函数归属与上表逐条一致，没有需要
调整的划分。P1.10 也已执行：测试归属与上表逐条一致，25 个测试函数名一个不多一个不少；两处
对 harness 归属的调整写在 §9 的 P1.10 记录里（`new_accumulation_space` 留 `http/tests.rs` 但保持
私有；`content_type` 只有一个消费者，跟着 `static_files` 的测试走，不进共享 harness，因此
`http/tests.rs` 对外的 harness 是 `accumulation_app` / `spawn_app` / `spawn_app_with_web_dir` 三个）。

### Phase 2 `store/lib.rs`（96 个 store 测试里的 46 个 + 5 个兄弟模块的调用点）

**项清单基线**（`--list-items`，2026-09-16 实测；口径：`定义项` = `fn`/`struct`/`enum`/`trait`/
`const`/`static`/`type` 等项数，`项内行` = 这些项占的正文行（含文档注释），`项外自由行` =
容器头/闭合括号与没归属的注释行，`接线行` = 总行 − 项内 − 项外，即 `use` 语句、`mod x;`、空行）。
这张表是本阶段的施工目标：每批的判定是「定义项数前后相同 + 项内行相同 + 残留 0」，所以改前改后
都该对得上这张表里对应行的数字。

| 文件 | 总行 | 定义项 | 项内行 | 项外自由行 | 接线行 |
| --- | ---: | ---: | ---: | ---: | ---: |
| `lib.rs` | 7,443 | 123 | 6,764 | 180 | 499 |
| `preparation.rs`（Phase 3） | 13,205 | 290 | 11,634 | 360 | 1,211 |
| `knowledge.rs` | 2,704 | 113 | 2,172 | 203 | 329 |
| `model_requests.rs` | 1,942 | 58 | 1,600 | 126 | 216 |
| `recycle.rs` | 1,257 | 49 | 962 | 73 | 222 |
| `version_copy.rs` | 1,061 | 17 | 938 | 8 | 115 |
| `dashboard.rs` | 902 | 21 | 752 | 31 | 119 |
| `prompts.rs` | 830 | 28 | 675 | 43 | 112 |
| `system_operations.rs` | 635 | 25 | 514 | 42 | 79 |
| `manuscript.rs` | 587 | 19 | 456 | 37 | 94 |
| `project_settings.rs` | 553 | 18 | 466 | 19 | 68 |
| `protocol_correction.rs` | 547 | 16 | 363 | 99 | 85 |
| `settings.rs` | 481 | 20 | 364 | 57 | 60 |
| `prompt_templates.rs` | 438 | 16 | 338 | 33 | 67 |
| `schema.rs` | 398 | 6 | 362 | 5 | 31 |
| `agent_messages.rs` | 374 | 18 | 279 | 39 | 56 |
| `budget.rs` | 319 | 10 | 238 | 25 | 56 |
| `export.rs` | 252 | 8 | 186 | 13 | 53 |
| `event_types.rs` | 166 | 11 | 132 | 17 | 17 |
| `event_log.rs` | 158 | 11 | 103 | 30 | 25 |
| `chapters.rs` | 126 | 5 | 79 | 14 | 33 |
| `growth.rs` | 112 | 4 | 74 | 18 | 20 |
| `entity/*.rs`（28 个文件） | 817 | 27 | 588 | 82 | 147 |
| 合计（50 个文件） | 35,307 | 913 | 30,039 | 1,554 | 3,714 |

`lib.rs` 的生产/测试各自切出多少项（生产区 = 1–1,437 行，测试区 = `#[cfg(test)] mod tests`
的 1,438–7,443 行）：生产区 **70 项 / 项内 1,073 行 / 项外 104 行 / 接线 260 行**，测试区
**53 项 / 项内 5,691 行 / 项外 76 行 / 接线 239 行**。也就是说 6,005 行的测试模块在项级只占 53 个
定义项（大头是 `#[tokio::test]` 函数与它们共用的 harness），Phase 2 的 P2.1 / P2.2 两批只动生产
区的 70 项（其中 P2.1 动 17 项、P2.2 动 **28 项 / 项内 661 行**——两批合计 45 项，生产区由此
只剩 25 项 / 项内 282 行），P2.3 才碰这 53 项。

| 批次 | 移动内容 | 目标 | 当前行参考 | 预估行 | 备注 |
| --- | --- | --- | --- | ---: | --- |
| P2.1 | `Command` + `impl Command`、`Notify` + impl、`CommandReceipt`、`payload_hash`、`replay_or_conflict`、`insert_receipt`、`replay_after_receipt_conflict`（含 `impl Store` 写法）、`db_id` `parse_id` `parse_optional_id` `transaction_error` `approval_mode_name` | `store/command.rs` | 255–261、866–900、1095–1114、1203–1226、1289–1313、1314–1437 | **已执行 250 行**（17 个定义项 / 项内 130 行） | `lib.rs` 补 `pub(crate) use command::{...}`（10 个名字）+ 私有 `use command::CommandReceipt;`，兄弟模块的 `use super::{...}` 一行不改；`replay_after_receipt_conflict` 保持私有，`CommandReceipt` 的 4 个字段改成 `pub(crate)`（`lib.rs` 的 `find_receipt` 要构造它）。现状 → 见 §9 |
| P2.2 | 项目与稿本：`list_projects` `project_progress_by_version` `get_project` `list_versions` `get_version_snapshot` `create_project` `create_version` `select_version` `project_id_for_version` `project_id_for_run` `project_id_for_chapter` `project_id_for_review` `project_exists` `find_receipt`、三个 payload 结构、`version_snapshot`、`project_bundle_from_models` `version_from_model` `project_from_model` `workflow_from_model`、`insert_project_bundle` `insert_novel_version` `insert_document_slots` `insert_workflow` | `store/projects.rs` | 326–390、390–937、901–931、932–978、1115–1202、1227–1288 | **已执行 827 行**（28 个定义项 / 项内 661 行） | `insert_novel_version` 等被 `version_copy.rs` 用，靠 `pub(crate) use` 保持 `super::` 路径；`CredentialPatch` 与它的 `Deserialize` 留在 `lib.rs`（对外可见类型）。现状 → 见 §9 |
| P2.3 | 6,005 行测试模块改成 `#[cfg(test)] mod tests;` + `src/tests/`：`mod.rs` 放 `use super::*` 与共享 harness（`functions()`、`in_flight_preparation_run`、`delegated_background_review`、`drafting_chapter_body_run`、`dispatched_body_tool_call`、`persisted_protocol_error_count`、`published_protocol_error_count`），其余按域拆 | `store/src/tests/` | 1438–7443 | **已执行：`lib.rs` 432 行 + 15 个文件 / 6,081 行** | 实际是 **14 个域文件 + `mod.rs`**，不是计划里估的 8 个：`fold`（单项 924 行）必须单独成文件，才能守住「每个文件 ≤ 1,300 行」。**本批不是「只移动」**（rustfmt 会因缩进层级变化重排）——见 §9 |
| P2.4 | 形状测试的 `include_str!` 路径表改成新位置（并且其 `files` 数组要覆盖 `preparation/` 拆出来的所有文件） | `src/tests/shape.rs` | 2207–2257 | **已执行：9 条路径改成 `../…`（其中 2 条被 rustfmt 折成三行）** | 与 P2.3 同一批；`files` 数组要覆盖 `preparation/` 拆出来的所有文件这一条要等 Phase 3 之后才做 |

Phase 2 收尾形状（P2.3 收工后实测）：`lib.rs` **432 行**（生产区 429 行：`Store`、`StoreError`、
`SCHEMA_VERSION`、`open`/`close`/访问器、`mod` 与再导出、`CredentialPatch` 与两个设置 helper，
加一行空行与 `#[cfg(test)] mod tests;`），`src/tests/` **15 个文件 / 6,081 行**，最大 `fold.rs`
**942 行**（≤ 1,300 行成立）。逐文件划分与「为什么是 14 个域文件」见 §9 的 P2.3 记录。
（P2.1 时这里的估值是「≈ 320 行」，那是按 1,224 行生产区的估计；P2.2 实测剩余 429 行——差在
`StoreError`（141 行）、`impl Store` 的打开/关闭链（64 行）与 13 条再导出比预估占行。）

### Phase 3 `store/preparation.rs` → `store/preparation/`（最大、安全网最薄）

先做两件「全 crate 都会受益」的事，再做按域的搬迁。

**Phase 3 的入口条件（2026-09-16 定下来，免得执行时再决定一次）**

1. Phase 2 收尾形状**已达成**（`lib.rs` 432 行、`src/tests/` 最大 942 行），
   `/tmp/nw-split-state/P2.3` 批次快照就位（111 个 Rust 源文件）；
2. §3 的 **V0 阶段基线**已在 P2.3 收工时跑过一遍（四条命令的实测数字记在 §9 的 P2.3 记录里，
   也是 Phase 3 的起点基线）：`cargo fmt --all -- --check` 通过、`cargo clippy --workspace
   --all-targets` **0 warning / 0 error**、`cargo test --workspace` **269 通过 / 0 失败**、
   `cargo test -p nekowriter-store -- --list` **96 个用例**。V0 是**测量基线**，它不新增测试；
3. 本阶段的保护只有三层，把它们当成预算来花：项级 I1/I2（§1）、**每批都跑**
   `cargo test -p nekowriter-store -p nekowriter-app`、以及上面那张项清单基线（`preparation.rs`
   **290 项 / 项内 11,634 行**）；
4. **批次粒度（2026-09-16 重算并定案，见 §9 的「Phase 3 批次粒度」）**：
   **一个目标模块 = 一层 = 一批**，即下表 P3.3–P3.17 每行各一批（P3.12 那一行 ~2,200 行拆成
   2 次提交），加上 P3.1 与 P3.2，**预计 18 批**，单批项内行数 40–1,200、目标文件 ≤ 1,300 行。
   敢放到这个尺寸的理由：项级判据**点名到项**（`[async fn foo(…)] 行序改变`）、`--list-items`
   的定义名双向差集、以及编译错误里的项名加行号，这三样都不随批量变大而失焦——所以批量只影响
   「回滚粒度」，不影响「失败定位」。**若某批的失败定位真的变难**（一编译就是一片错、判据报的
   项名互相掩盖），就地缩到那批里最小的可独立编译子集：先按项名二分，再缩到「单项 + 它的私有
   依赖闭包」，缩下来的子集继续用同一个基准快照，不重跑整栈核对；
5. **如果要在动 `preparation.rs` 之前补一层端到端安全网（把准备→细纲→正文的完整流程固化成
   测试），那是一件需要单独决策的新工作**：现行计划里没有「给 `preparation.rs` 写测试」的批次，
   §2 只记下了「自身 0 个测试，所以放最后」这个事实。不动它就是接受「靠上面三层保护」，动它
   就先写测试再拆。

| 批次 | 移动内容 | 目标 | 当前行参考 | 预估行 |
| --- | --- | --- | --- | ---: |
| P3.1 | `insert_event` / `insert_app_event` / `insert_event_row`（事件写入正典） | **已执行**：`store/event_log.rs`（与读侧同模块，偏离原「新建 `store/events.rs`」的理由见 §9）+ `preparation.rs` 里 `pub(crate) use crate::event_log::{insert_event, insert_app_event};`（调用方按 `preparation::…` 引用，所以再导出必须在 `preparation.rs`，不是 `lib.rs`） | 13142–13205 | 64（已完成：1 空行 + 63 行代码） |
| P3.2 | `preparation.rs` → `preparation/mod.rs` 纯改名（`lib.rs` 的 `mod preparation;` 不动）；同行修 `tests/shape.rs` 的 `include_str!` 路径与显示名（P2.4 的同类修正）。括号里原来的「`mod.rs` 只留模块文档、imports、常量与 payload、再导出」是 **P3.17 之后的终态**，不是本批（本批 `mod.rs` 与改名前的 `preparation.rs` 逐字节相同） | `preparation/mod.rs`（新建）+ `tests/shape.rs` | 全文 | **已执行：0 行代码搬迁**（13,144 行原样换路径）|
| P3.3 | 引用与意见：`validate_chapter_body_feedback_citations` `validate_document_citation` `validate_review_decision_citations` `validate_review_decision_feedback` `held_candidate_for_feedback_citation` `proposal_candidate_for_feedback_citation` `validate_text_citation_range` `byte_offset_for_unicode_scalar` `feedback_citations_from_json` `resolve_pending_candidate_feedback_citations`；`FeedbackBatchAnchors` `feedback_status_name` `feedback_status_from_db` `feedback_batch_anchors` `QueuedChapterBodyFeedback` `queue_chapter_body_feedback` `chapter_body_run_status_matches_execution` `writing_run_status_for_execution` `bind_pending_feedback_to_tool` `latest_unsealed_chapter_body_tool_result` `bind_pending_feedback_to_existing_tool_for_pause` | `preparation/citations.rs` | **已执行：原 633–1267** | **已执行：635 行（565 行代码 + 49 行注释 + 21 行空行）→ `citations.rs` 667 行** |
| P3.4 | 工具体类型与参数解析：`ChapterBodyMutation` `AgentChapterBodyMutation` `AgentBodyToolError` + impl、`ChapterBodyAgentToolCall` `AppliedChapterBodyToolCall` `AgentQuestionToolCall` `AppliedAgentQuestionToolCall`、`Raw*ToolArguments`、`validate_agent_*_tool_call` `parse_agent_*`、`*_tool_error_result`、`HeldChapterBodyToolPayload` `held_chapter_body_tool_payload_from_model` `pending_chapter_body_proposal_from_tool_call` `candidate_content_hash` | `preparation/tools/model.rs` | **已执行：原三段 229–285、358–564、577–646** | **已执行：334 行（三段）→ `tools/model.rs` 363 行 + `tools/mod.rs` 6 行** |
| P3.5 | 自由工具与提问工具：`FreeAgentToolCall` `AppliedFreeAgentToolCall` `FreeAgentToolContext` `FreeAgentToolResult`、`prepare_free_agent_tool_call` `commit_free_agent_tool_call` `next_agent_tool_sequence`；`impl Store` 的 `execute_agent_question_tool_call` `free_agent_tool_result` `execute_free_agent_tool_call` | **已执行**：`preparation/tools/free.rs`（+ `tools/mod.rs` 加一行 `pub(crate) mod free;`）| **已执行：原两块 329–676、3111–3566** | **已执行：804 行（348 + 456）→ `tools/free.rs` 844 行** |
| P3.6 | 正文工具与暂停边界：`ResolvedHeldChapterBodyTool` `resolve_held_chapter_body_tool` `insert_resolved_held_chapter_body_tool_events` `ResolvedHeldChapterBodySubmission` `resolve_held_chapter_body_submission` `insert_resolved_held_chapter_body_submission_events` `HeldChapterBodyPauseBoundary` `held_chapter_body_pause_boundary`、`CompletedRunContextStage` `CompletedRunContextFold`（+ P3.4 记录里预分配的 `HeldChapterBodySubmissionCheckpoint`） | **已执行**：`preparation/tools/held.rs`（+ `tools/mod.rs` 加一行 `pub(crate) mod held;`）| **已执行：原三段 322–331、333–934、946–1007** | **已执行：674 行（10 + 602 + 62）→ `tools/held.rs` 718 行** |
| P3.7 | 准备与细纲命令：`get_preparation_snapshot` `list_completed_run_context_folds` `save_novel_brief` `start_preparation` `submit_preparation_artifacts` `decide_preparation_review` `get_chapter_guidance` `save_chapter_guidance` `start_chapter_outline` `get_chapter_outline`；**本层独占的 5 个 `*_METHOD` 常量与 5 个私有 payload DTO 随命令一起搬**（计划行原来没列，理由见 §9）| **已执行**：`preparation/commands.rs`（+ `mod.rs` 加一行 `mod commands;`）| **已执行：原方法区 343–1044（10 项 / 702 行）+ 常量行 114、115、116、118、119 + payload 块 143–146、148–151、153–158、167–171、173–176** | **已执行：`mod.rs` 删 735 行 → `commands.rs` 775 行（20 项）** |
| P3.8 | 正文流转与意见：`set_approval_mode` `request_chapter_body_pause` `resume_chapter_body_pause` `stop_writing_run` `continue_writing_run` `submit_run_feedback` `list_materialized_chapter_body_feedback` `list_unsealed_agent_tool_result_provider_ids` `answer_user_question` + 结果类型（`QuestionAnswerResult` `StopWritingRunResult` `InterruptionSource` 与它的两个 `impl`、`InterruptionOutcome` `MaterializedChapterBodyFeedback`）；**本层独占的 7 个 `*_METHOD` 常量与 4 个私有 payload DTO 随层一起搬**（P3.7 规则 ①）| **已执行**：`preparation/body/control.rs`（+ `body/mod.rs` 加一行 `pub(crate) mod control;`）| **已执行：原方法区 311–1596（9 项 / 1,286 行）+ 类型区 229–298（7 项 / 70 行）+ 常量行 120、122–127 + payload 块 165–169、178–183、185–189、191–194** | **已执行：27 项 / 1,388 行 → `mod.rs` 10,000 → 8,617、`control.rs` 1,439、`body/mod.rs` 7** |
| P3.9 | 正文写入与提交：`get_chapter_body` `get_document_revision` `append_chapter_body` `replace_chapter_body` `execute_chapter_body_agent_tool_call`（678 行单函数）`decide_chapter_body_proposal`；**本层独占的 3 个 `*_METHOD` 常量与 3 个私有 payload DTO 随层一起搬**（P3.7 规则 ①）| **已执行**：`preparation/body/edit.rs`（+ `body/mod.rs` 加一行 `pub(crate) mod edit;`）| **已执行：原方法区 215–1380（6 项 / 1,161 行）+ 常量行 124–126 + payload 块 151–155、157–162、164–169** | **已执行：12 项 / 1,181 行（连尾随空行 1,190 行）→ `mod.rs` 8,617 → 7,423、`edit.rs` 1,238** |
| P3.10 | 终稿提交：`commit_chapter_body_model_submission` + 它的协议纠正常量与分支 | `preparation/body/commit.rs` | 现行 2765–3150（1/1 项，共 386 行） | ~570 |
| P3.11 | 审核命令：`submit_chapter_final_review` `decide_chapter_final_review` `add_extra_chapter` `advance_to_next_chapter` `confirm_novel_completion` `submit_chapter_outline` `decide_chapter_outline_review` `decide_review` | `preparation/reviews/decide.rs` | 现行 3339–4541（8/8 项，共 1,196 行） | ~1,200 |
| P3.12 | 审核委托：`delegate_review` `cancel_review_delegation` `delegated_review_task` `commit_delegated_review_result` `fail_delegated_review_attempt`、`impl DelegatedReviewKind`、`validate_delegation_notes` `validate_delegation_citations` `delegatable_review_target` `replayed_delegated_review_result` `review_stage_matches` `parent_run_matches_review_kind` `validate_review_target_is_current` `insert_delegated_review_run` `build_delegated_review_task` `delegated_review_notes` `delegated_review_citations`、`DelegatedReviewKind` `DelegatedReviewTask` `ReviewDelegationResult` `CancelReviewDelegationResult` | `preparation/reviews/delegate.rs` | 现行 4543–5105、5427–6096、6949–6994（20/21 项，共 1,124 行） | ~2,200（拆成 2 次提交） |
| P3.13 | 快照读取：`revision_content` `approved_preparation_context` `delegated_chapter_context` `chapter_and_guidance_document` `chapter_and_document` `next_chapter_after` `all_main_chapters_are_approved` `ensure_no_active_top_level_write_run` `current_chapter_guidance` `current_chapter_outline` `current_chapter_body` `chapter_body_snapshot` `pending_user_question` `pending_chapter_body_proposal` `pending_chapter_edit_review` `latest_chapter_edit_review` `held_chapter_body_tool` `held_chapter_body_submission` `pending_chapter_body_feedback`(+`_from_model`) `delivered_chapter_body_feedback`(+`_from_model`) `latest_chapter_outline_fold` `latest_chapter_body_fold` | `preparation/snapshot.rs` | 现行 6098–6943（22/24 项，共 698 行） | ~850 |
| P3.14 | 审核目标与审核行：`apply_chapter_edit_review_decision` `copied_chapter_fold` `review_type_for_id` `validate_preparation_submission` `preparation_upstreams` `dependency_revision_ids` `depends_on_any_changed_kind` `insert_revision_dependency` `insert_preparation_review` `insert_chapter_outline_review` `insert_chapter_final_review` `insert_chapter_edit_review` `mark_review_stale`、4 组 `*ReviewTarget` + 4 个 `*_review_target` `preparation_kind_from_role` `persist_human_review_decision` | `preparation/reviews/targets.rs` | 现行 7010–7368、7611–7930、8204–8248（15/17 项，共 712 行） | ~1,230 |
| P3.15 | 委托结论落账：`validate_delegated_review_decision` `delegated_review_decision_for_workflow` `persist_delegated_review_decision` `apply_delegated_preparation_review` `apply_delegated_chapter_outline_review` `apply_delegated_chapter_final_review` `current_preparation_is_approved` | `preparation/reviews/apply.rs` | 现行 8250–8891（7/7 项，共 636 行） | ~610 |
| P3.16 | 依赖与折叠：`chapter_outline_dependency_revision_ids` `chapter_body_outline_dependency_revision_id` `chapter_body_revision_dependency_ids` `revision_dependency_ids` `build_preparation_context_fold` `build_chapter_outline_context_fold` `build_chapter_body_context_fold` `materialize_chapter_plan` `latest_run_message_sequence` `chapter_outline_run` `update_chapter_outline_run_status` `chapter_body_run` `update_chapter_body_run_status` `current_novel_brief` `current_preparation_artifacts` `latest_preparation_fold` `completed_run_context_folds` | `preparation/folds.rs` | 现行 8893–9637（17/17 项，共 618 行） | ~700 |
| P3.17 | 模型↔域转换与行插入：`review_confidence_from_model` `preparation_review_from_model` `chapter_outline_review_from_model` `chapter_final_review_from_model` `chapter_edit_review_from_model` `review_status_from_db` `insert_document_revision`(+`_with_payload` / `_and_source`) `project_id_for_version` `insert_preparation_run` `insert_chapter_outline_run` `insert_chapter_body_run` | `preparation/rows.rs` | 现行 9639–9982（11/13 项，共 267 行） | ~440 |

**「当前行参考」列的读法（P3.9 之后重新实测，`preparation/mod.rs` 现行 7,423 行）**：区间是按项名
逐个定位后给出的连续段；括号里「n/m 项」= 「在现行 mod.rs 里按名字找到的项数 / 该行列出的名字数」，
差的几个是计划里的通配符写法（`Raw*ToolArguments`、`parse_agent_*`、`impl Store`、`*ReviewTarget`、
`_with_payload` 之类），执行时一律**按项名或按定义位置找，不照行号搬**。也正因为如此，
**本阶段的行号不是互不重叠的块**：P3.12（4543–5105、5427–6096、6949–6994）与 P3.13（6098–6943）
仍然交错，**且 P3.9 之后所有大于 1381 的旧行号都要再减 1,194**（本批从 `mod.rs` 搬走的区间总量）。
**每批执行后这张表的行号都要重算一次**（重算脚本与口径见 §9 的 P3.3 记录）。

**P3.7 定下的两条执行规则（P3.8 起沿用）**：
① **DTO 与常量的归属**：私有 payload / `*_METHOD` 常量若只被本层的项使用，就随层一起搬（本批 5 + 5 个，
共 10 项）；若被本层与另一层共用（本批 `DecidePreparationReviewPayload`、`DECIDE_PREPARATION_REVIEW_METHOD`
还被 P3.11 的三个 `decide_*` 用），就留在 `mod.rs`，两层各自按 `super::…` 引用。
② **可见性**：`mod.rs` 自己的私有项对**子模块**（`preparation/commands.rs`）本来就可见，所以本批一个
`pub(crate)` 都没加；只有被**兄弟模块**（`citations`、`tools/*`）或别的 crate 引用时才需要（P3.3–P3.6
加的正是这一类）。**本批也不需要任何再导出**：搬走的都是 `Store` 的固有方法，调用方按方法语法调用，
这些名字从未出现在任何 `use` 里。

Phase 3 收尾形状：`preparation/mod.rs` ≤ 400 行，最大子模块 ≤ 1,300 行，没有单文件超过 2,000 行。

### Phase 4 收口（只有文档与注释）

| 批次 | 内容 | 校验 |
| --- | --- | --- |
| P4.1 | `docs/02` §2.1 模块表改成新事实（`store::command`、`store::events`、`store::projects`、`store::preparation::{commands,body,reviews,tools,snapshot,folds,rows,citations}`、`app::http::{error,dto,gate,static_files,state,handlers::*}`、`app::startup`）；§2.4 第 2 条改写为「已完成 + 剩余项（scheduler 按 lane、core::lib）」 | 模块表与 `crates/*/src` 逐行对账 |
| P4.2 | `Cargo.toml` 里点名 `preparation.rs` / `lib.rs` 的 lint 注释按新文件名更新 | `cargo clippy --workspace --all-targets` 不变 |
| P4.3 | 本文件补「结果」小节：每阶段前后行数、每批提交哈希、clippy 与测试的基线 diff 结论 | `cargo test --workspace` |

---

## 6. 完成定义

1. 三个目标文件都不再是万行级：`store/lib.rs` ≤ 350 行（测试在 `src/tests/` 下按域分文件，
   最大文件 ≤ 1,300 行），`app/main.rs` ≤ 150 行（只算生产代码：导入表与 4 条再导出 + `main`；
   P1.10 收尾后实测 **115 行**——计划最初的「≤ 100 行」是拆分开始前的估值，28 行导入表与
   4 条再导出加进去就超了），`preparation/mod.rs` ≤ 400 行且最大子模块 ≤ 1,300 行。

   **Phase 1 的验收点（P1.10 收尾，Phase 1 已完成）**：`crates/nekowriter-app/src/main.rs`
   只剩导入表、4 条 `pub(crate) use` 再导出（`main.rs:25–28`）与 `fn main()`（`30–115`），
   实测 **115 行**、定义数 **1**（只剩 `main`）；原 `#[cfg(test)] mod tests`（1,179 行、25 个测试）
   已按 §5 的归属表分散到 8 个模块，`main.rs` 里不再有任何测试，二进制入口与 `router`/`api_not_found`/
   `shutdown_service`（`http/mod.rs`）、各域 handler（`http/handlers/<域>.rs`）之间的接线全部是
   `pub(crate) use`，因此没有一处调用点被改写；
   `http/mod.rs` 里那条 `use crate::{...}` 只剩 **五个域模块**
   （`dashboard` `knowledge` `manuscript` `recycle` `templates`）——每个名字都被 `router` 的
   路由表引用（1 / 19 / 4 / 8 / 6 次），没有任何可以再删的名字；所有 handler 名字都改成从
   `crate::http::handlers::<域>` 取，而路由表每个 `.route(...)` 从 P1.5 到现在一字未改。
2. 每一批都满足 §1 的 I1 / I2，diff 范围只落在被拆文件与新模块文件里；测试数只增不减，
   clippy 无新增 warning。
3. `docs/02` §2.1 的模块表与文件系统一致；`docs/05` F-09、`docs/06` F-09 在 `docs/08` 里被
   引用为「已执行」。
4. 三个阶段各自可独立回滚：阶段内每批一次提交，阶段末一个 tag。

## 7. 提交粒度与回滚

- 一个批次一次提交，提交信息写明「批次号 + 移动了什么 + 本批点名的连带改动」；不使用
  `git add -A`，避免把并行工作的改动卷进来（本仓是共享检出）。
- 回滚粒度是单批：`git revert <该批>`。阶段级回滚用阶段末的 tag 对照检查。
- 每批完成后立刻跑 V1 四道；失败就地修（改回搬迁方式），不把失败批次留给下一批。

## 8. 明确不做

- **`app/scheduler.rs` 按 lane 拆分**：`docs/05` 建议的另一半天，但它是「同一份策略的两个
  所有者」问题的延伸，需要在 `ProtocolCorrectionTarget` 的现有形状上做判断，属于设计变更，
  不放进纯搬迁轮。
- **`ApiError::into_response` 按错误域分派到小函数**：P1.1 只把它整块搬走（466 行 match 的
  match 臂逐字不动）；把它拆成 `run/review/knowledge/chapter/settings` 若干函数是**行为等价
  重写**，要件是「每个错误码的状态码、文案、`details` 形状逐个对照」，应当单独一轮。
- **`core/lib.rs`（3,748 行）**：`core::workflow` / `core::document` 的切缝同样存在，但 core
  是决策层、测试最密（38 个单测 + 各 crate 的间接覆盖），放在 store/app 之后按同样的协议做。
- **测试与实现的同置**：本计划把 `lib.rs` 的测试搬进 `src/tests/`，但没有把它们进一步塞进
  被测试模块旁边（像 `knowledge.rs` 那样自置测试）。「每个模块自带测试」是另一件事，需要先
  决定 `Store::open(TempDir)` 这种跨模块 harness 的归属。

---

## 9. 执行记录

每完成一批就追加一段：日期、批次号、改动文件与搬迁行数、机械结论、门禁数字。这一节是
「这一批到底动了什么」的唯一出口；`docs/02` §2.1 的模块表按计划在 P4.1 一次性对齐，
不每批改一次。

**提交口径（2026-09-16 补记）**：本节的每条批次记录都写着「（未提交）」，那是「**没有单批提交**」——
Phase 1 到 P3.9 的全部改动于 `7148fbb`（`refactor: split the three giant files into layer modules`，
47 个文件 / +21,920 / −17,044）一次性入库；§7 要求的单批回滚粒度在这批里由
`/tmp/nw-split-state/P1.2 … P3.9` 的快照与本节的逐批数字保留，后续批次（P3.10 起）恢复「一批一提交」。

**读数字前先看口径**：本会话 2026-09-16 把 I2 从「行池化多重集」改成「按定义项逐项有序比对」
（见 §3 与本节最后一条记录）。各处记的「归一化代码行 X → Y」是**旧口径**的数字，新工具不再产出
这一行（改报「定义项 N 个 / 项内 M 行 / 项外自由行 K」），两套数字**不可互比**；旧数字作为
「当时那版判据算出的结论」仍然成立，且结论已在新判据下用快照逐批重跑复核。

### 2026-09-15 P0.1 + P0.2：验收工具落地

- 新增 `cargo xtask check-move-only`（`crates/xtask/src/main.rs`，+551 行含 7 条用例）。
  没有新依赖，也没有新能力：它只读 git 与工作区文件，把 §1 的 I1 / I2 算成结论。
- `cargo test -p xtask`：**11 通过 / 0 失败**（基线 4 + 新增 7）；
  `cargo clippy -p xtask --all-targets`：**0 warning**。
- 反向验证（`--base HEAD`，拿改过一行的 `main.rs` 副本当 `--into`，副本在 `/tmp`、不进仓库）：
  退出码 **1**，逐行点名 `- const DEFAULT_PORT: u16 = 47_821;` /
  `+ const DEFAULT_PORT: u16 = 47_822;`，结论 `I2 不成立：2 行不是搬迁`。
- 正向对照（同一份文件对同一份文件）：`MOVE-ONLY OK`，退出码 **0**。
- 落地过程中改了两次判据，两次都是被真实搬迁逼出来的：
  1. `use` 语句要**整条**算接线——rustfmt 按名字长度折行，只丢行首会把折行差异报成代码差异
     （P1.1 第一次跑就是这 19 行残留）；
  2. **新增注释**放行、**删除注释**仍算残留——新模块需要自己的文档头，而丢文档不是搬迁。
- 未验证项：`impl Store` 跨模块 + 私有字段访问（§4 第 2 条）在 P1.1 里只碰到了
  `impl IntoResponse for ApiError` 这一种形状；`impl Store` 那一种留到 P2.1 首次跑时确认。

### 2026-09-15 P1.1：`ApiError` → `app::http::error`

- 声明：`--from crates/nekowriter-app/src/main.rs`，`--into` 上述文件加
  `crates/nekowriter-app/src/http/mod.rs`、`crates/nekowriter-app/src/http/error.rs`
  （默认作用域 `crates/nekowriter-app`）。
- 机械结论：`MOVE-ONLY OK`。改动文件 3 个，全在允许集合里（作用域外另有 2 个路径：另一条
  线程的工作与本文档，不参与判定）；`main.rs` **+9 −604**、`http/mod.rs` **+9**（新建）、
  `http/error.rs` **+620**（新建）；归一化代码行 **3,890 → 3,903**，残留 **0**，放行新增注释
  **13 行**（两个新文件的模块文档头）。
- 搬走的东西：`ErrorEnvelope`、`ErrorBody`、`ApiError`（16 个变体）、12 个 `From` 实现、
  `Display`、`IntoResponse`（466 行 match），共 595 行（`main.rs:470–1064`）。
- 原处只剩两行接线：`mod http;`（模块声明表按字母序）与
  `pub(crate) use http::error::ApiError;`。因此 `crate::ApiError` 的 6 个调用方
  （`knowledge.rs`、`recycle.rs`、`templates.rs`、`dashboard.rs`、`manuscript.rs`、
  `prompts.rs`）一字未改——`git status` 也证明它们没有被触碰。
- 归一化覆盖的可见性/接线改动：`pub(crate) mod error;`、`pub(crate) enum ApiError`、两条
  `pub(crate) use`。另有三处 `use` 行变化（由编译器的 unused-import 提示给出）：`main.rs` 的
  `nekowriter_core` 导入表去掉 7 个只被搬走代码使用的名字、删掉
  `use nekowriter_core::recycle::RecycleQueryError;`、把只被测试用到的 `Value` 下沉进测试
  模块；`http/error.rs` 不需要 `std::fmt::Write as _`（`Formatter` 有固有的 `write_str` /
  `write_fmt`）。**没有一行函数体变化。**
- 门禁（2026-09-15）：`cargo fmt --all -- --check` 通过；逐包 `cargo clippy --all-targets`
  六个 crate 全部 **0 warning / 0 error**；`cargo clippy --workspace --all-targets`
  **0 warning / 0 error**；`cargo test --workspace` **260 通过 / 0 失败**（agent 44、
  app 67 + 集成 2、core 38、prompt-lab 2、store 96、xtask 11）；`pnpm lint` **0 问题**、
  `pnpm test` **10 通过 / 0 失败**。
- 未提交：本轮只做搬迁与验证，提交留给维护者决定（回滚粒度仍是「一批一提交」，见 §7）。

### 2026-09-15 P1.2：HTTP DTO → `app::http::dto`

- 声明：`--from crates/nekowriter-app/src/main.rs`，`--into` 上述文件加 `http/mod.rs`、
  `http/dto.rs`；error.rs 作为上一批的产物一并声明在 `--into` 里。
- 搬走的东西：**39 个 DTO**（31 个 `*Request`、5 个 `*Response`、3 个查询/视图类型
  `EventsQuery` / `ExportQuery` / `PublicSystemModelSettings`），包括嵌套的
  `PreparationArtifactRequest` 与 `ChapterPlanDraftRequest`。字段、`serde` 属性与文档一行未改，
  只有可见性从私有放宽到 `pub(crate)`（类型与字段都要跨模块用）。
- `main.rs` 用一条 `use crate::http::dto::{...};` 把名字接回原位，所以每个 handler、每个路由、
  测试模块都保持字面不变；归一化覆盖的三处 `use` 行变化：新增 dto 导入表、
  `nekowriter_core` 列表去掉 8 个只被 DTO 使用的名字、删掉 `use serde::{Deserialize, Serialize};`
  （它现在只在 `dto.rs` 里用得到）。
- 机械结论分两层：
  1. **相对提交点的整栈核对**（`--base HEAD`，`--from main.rs`，`--into` 四个文件）：
     `MOVE-ONLY OK`，归一化 **3,890 → 3,909 行**、残留 **0**、放行新增注释 **19 行**；
     逐文件 `main.rs +25 −967`、`http/mod.rs +10`、`http/dto.rs +372`、`http/error.rs +620`。
  2. **单批隔离**用本批新增的批次状态：对着 P1.2 快照做恒等核对 → `MOVE-ONLY OK`、残留 0；
     反向验证（把快照副本里 `dto.rs` 的 `event_watermark: String,` 改成 `u64,`）→
     `声明内改动：crates/nekowriter-app/src/http/dto.rs`、残留 2 行、退出码 1。
- **判据缺口与修法**（本批被真实状态逼出来的第二处）：`--base HEAD` 在上一批未提交时
  **无法隔离单批**——HEAD 的 `main.rs` 里还有 `ApiError`，第一次跑就把上一批的 595 行算成
  残留（565 行）并把 `http/error.rs` 报成声明外改动，退出码 1。工具因此补了
  `--base-state <目录>` / `--write-state <目录>`：把 `crates/` 下的 Rust 源码存成快照，下一批
  对着快照读基准内容并算「哪些文件变了」，不依赖提交点、也不受别的 crate 并行改动影响。
  新增用例覆盖「空快照 = 全新增、写入后 = 零改动、改一行与删一个文件各自被认出来」。
- 门禁（2026-09-15）：`cargo fmt --all -- --check` 通过；逐包 `cargo clippy --all-targets`
  六个 crate 全部 **0 warning / 0 error**；`cargo clippy --workspace --all-targets`
  **0 warning / 0 error**；`cargo test --workspace` **261 通过 / 0 失败**（agent 44、
  app 67 + 集成 2、core 38、prompt-lab 2、store 96、xtask 12）；`pnpm lint` **0 问题**、
  `pnpm test` **10 通过 / 0 失败**。
- 本批文件清单：`crates/nekowriter-app/src/main.rs`、`crates/nekowriter-app/src/http/mod.rs`、
  `crates/nekowriter-app/src/http/dto.rs`（新建，372 行）、`crates/xtask/src/main.rs`（工具）。
  未提交：同 P1.1；下一批用 `/tmp/nw-split-state/P1.2` 作为基准状态。

### 2026-09-16 P1.3：启动链 → `app::startup`

- 声明：`--from crates/nekowriter-app/src/main.rs`，`--into` 同一文件加
  `crates/nekowriter-app/src/startup.rs`。
- 搬走的东西：`DEFAULT_PORT`、`Cli`（含 4 个 clap 字段）、`StartupError`（6 个变体）、
  `ProcessLock` + `acquire`、`StartupStep` + `label`/`hint`、`startup_failure`、
  `startup_failure_reason`、`startup_failure_message`、`Startup`（5 个字段）、`prepare`（5 步）、
  `default_data_dir`、`resolve_data_dir`、`default_web_dir`——共 322 行（估算 ~410 行，实际更小）。
- `main.rs` 原处只剩接线：`mod startup;` 与 `use startup::{Cli, StartupError, prepare};`，
  所以 `#[tokio::main] async fn main()` 的每一行（`Cli::parse()`、`prepare(&cli).await?`、
  `startup.store.clone()` / `data_dir` / `web_dir`、`cli.port`、`cli.no_browser`）字面不变，
  进程退出码与 stderr 形态也不变。测试模块加一条
  `use crate::startup::{StartupStep, default_web_dir, startup_failure_message};`：这三个名字只被
  测试使用，不进 crate 根的导入表（与 P1.1 把只被测试用的 `serde_json::Value` 下沉同一手法）。
- 可见性：`Cli` 的字段也要 `pub(crate)`（`main` 读 `cli.port` 与 `cli.no_browser`，第一次编译
  就报 4 处 `E0616`）；`ProcessLock`、`startup_failure`、`startup_failure_reason`、
  `default_data_dir`、`resolve_data_dir`、`DEFAULT_PORT` 保持私有。归一化覆盖这三类改动。
- 机械结论分两层：
  1. **单批隔离**（`--base-state /tmp/nw-split-state/P1.2`）：`MOVE-ONLY OK`，
     `main.rs` 3,298 → 2,996 行、`startup.rs` 0 → 322 行，归一化代码行 **3,005 → 3,010**、
     残留 **0**、放行新增注释 **5 行**（新模块的 5 行 `//!` 文档头）。工具报出的基准是快照里的
     3,298 行而不是 HEAD 的 4,240 行，这本身就说明 `--base-state` 路径真的在生效。
  2. **整栈核对**（`--base HEAD`，5 个 `--into`）：`MOVE-ONLY OK`，归一化
     **3,890 → 3,914 行**、残留 **0**、放行新增注释 **24 行**；逐文件 `main.rs +30 −1,274`、
     `startup.rs +322`、`http/mod.rs +10`、`http/dto.rs +372`、`http/error.rs +620`。
- 判据缺口：本批没有——P1.2 补的 `--base-state` 已经够用，工具一行未改。
- 额外机械证据（相对 HEAD 的未提交整栈，含未跟踪）：`git diff --name-only HEAD` +
  `git ls-files --others --exclude-standard` 共 **7 个路径**，逐项归类：
  `crates/nekowriter-app/src/main.rs`（被拆文件）、`crates/nekowriter-app/src/startup.rs`、
  `crates/nekowriter-app/src/http/mod.rs`、`http/dto.rs`、`http/error.rs`（新模块）、
  `crates/xtask/src/main.rs`（工具）、`docs/08-giant-file-split-plan.md`（文档）。
  **声明外 0 个**：`crates/nekowriter-app/src/` 下另外 10 个模块与 `crates/nekowriter-store/src/`
  的 15 个兄弟模块都不在清单里——调用方从 HEAD 以来一字未改。
- 门禁（2026-09-16）：`cargo fmt --all -- --check` 通过、`git diff --check` 通过；逐包
  `cargo clippy --all-targets` 六个 crate 全部 **0 warning / 0 error**；
  `cargo clippy --workspace --all-targets` **0 warning / 0 error**；`cargo test --workspace`
  **261 通过 / 0 失败**（agent 44、app 67 + 集成 2、core 38、prompt-lab 2、store 96、xtask 12）；
  `pnpm lint` **0 问题**、`pnpm test` **10 通过 / 0 失败**。
- 执行提醒（本批踩到一次）：强制全量重编译要用只碰已存在文件的
  `find crates -name '*.rs' -exec touch {} +`；本批一度按 crate 名字 `touch
  crates/<crate>/src/main.rs`，给三个只含 `lib` 的 crate 凭空造出 0 字节文件（cargo 因此把
  它们当成 bin target 报 `E0601`）。已全部删除并复核工作区回到上面的 7 个声明文件。
- 本批文件清单：`crates/nekowriter-app/src/main.rs`（3,298 → 2,996 行）、
  `crates/nekowriter-app/src/startup.rs`（新建，322 行）。
  未提交：同 P1.1～P1.2；**下一批的基准状态是 `/tmp/nw-split-state/P1.3`**（83 个 Rust 源文件）。

### 2026-09-16 P1.4：共享状态 → `app::http::state`

- 声明：`--from crates/nekowriter-app/src/main.rs`，`--into` 同一文件加
  `crates/nekowriter-app/src/http/state.rs`（新模块）与 `crates/nekowriter-app/src/http/mod.rs`
  （为它加一行 `pub(crate) mod state;`）。
- 搬走的东西：`AppState`（9 个字段，含 2 处字段文档）与 `EventStreamState`（7 个字段），
  共 24 行原文（`main.rs` 2,996 → 2,972 行），新模块 42 行。字段名、字段文档、字段类型
  一字未改；可见性从私有放宽到 `pub(crate)`（handler 与 SSE 都要按字段读）。
- 原处同样只留再导出，与 P1.1 同一手法：`pub(crate) use http::state::{AppState,
  EventStreamState};`。于是 `crate::AppState` 的 71 处引用、`crate::EventStreamState` 的 4 处，
  以及 5 个兄弟模块的 `use crate::{ApiError, AppState};`（`dashboard.rs`、`templates.rs`、
  `recycle.rs`、`manuscript.rs`、`knowledge.rs`）全部字面不变。
- 三处 `use` 行变化（都属于接线）：`Store` 从 crate 根列表下沉进测试模块的
  `nekowriter_store` 导入表（只被测试用），`watch` 从 `tokio::sync` 列表删掉（只被
  `EventStreamState` 用），`engine::AccumulationEngine` 从 crate 根删掉（只被 `AppState`
  的字段类型用）。
- 机械结论分两层：
  1. **单批隔离**（`--base-state /tmp/nw-split-state/P1.3`）：`MOVE-ONLY OK`，
     `main.rs` 2,996 → 2,972 行、`http/state.rs` 0 → 42 行、`http/mod.rs` 10 → 12 行；
     归一化代码行 **2,720 → 2,734**、残留 **0**、放行新增注释 **14 行** = `state.rs` 的 6 行模块文档头
     + `http/mod.rs` 的 8 行文档头（含本批新增的那 1 行）：`http/mod.rs` 不在 `--from` 里，
     所以它的注释行全部算新增；`state.rs` 里搬过去的两行字段文档与基准的多重集互相抵消。
  2. **整栈核对**（`--base HEAD`，6 个 `--into`）：`MOVE-ONLY OK`，归一化
     **3,890 → 3,921 行**、残留 **0**、放行新增注释 **31 行**；逐文件 `main.rs +34 −1,302`、
     `startup.rs +322`、`http/mod.rs +12`、`http/dto.rs +372`、`http/error.rs +620`、
     `http/state.rs +42`。
- **独立佐证（不依赖 xtask）**：用 `git show` + 逐文件归一化 awk + `sort` + `comm` 重做一遍，
  与工具的数字逐位吻合：单批隔离 before/after = **2,720 / 2,734**、base 独有 **0**、未匹配 **0**
  （after 独有 14 行全是注释）；整栈 before/after = **3,890 / 3,921**、未匹配 **0**
  （after 独有 31 行全是注释）。另做一条更粗的原始多重集比对（不丢任何行，只按行首分类）：
  从 HEAD 的 `main.rs` 里删掉的行中，在新模块找不到的只有 **15 行，全部是导入表接线**——
  12 行是改写过的多行 `use` 块的续行（`nekowriter_core` / `nekowriter_store` / `nekowriter_agent`
  的名单），3 行是整条被删或改写的 `use`（`use tracing::{error, info, warn};`、
  `use nekowriter_core::recycle::RecycleQueryError;`、
  `use nekowriter_store::{CredentialPatch, KnowledgeSpaceInput, NewEmbeddingSettings};`——
  最后一条在测试模块里多了 `, Store`）；**代码行 0**，与工具的 `残留 0` 一致。
- 这一轮的判据缺口出在**佐证脚本**而不是仓库工具上（记录在案，免得下次重踩）：① 归一化 awk
  把行首 `use` 当前缀匹配，`user: request.user,` 也被当成 use 语句开头，吞掉了后面若干行；
  改成要求 `use` 后面跟空白或 `(`。② 一只 awk 状态机跨文件串联时，前一个文件没闭合的状态
  吞掉了 `error.rs` 的文档头；改成逐文件独立归一。修好后两侧数字与工具完全一致——这也是对
  xtask 那份实现的一次外部交叉验证。
- 额外机械证据（相对 HEAD 的未提交整栈，含未跟踪）：**8 个路径** = 被拆文件
  `crates/nekowriter-app/src/main.rs`；新模块 `crates/nekowriter-app/src/startup.rs`、
  `http/mod.rs`、`http/dto.rs`、`http/error.rs`、`http/state.rs`；工具 `crates/xtask/src/main.rs`；
  文档 `docs/08-giant-file-split-plan.md`。**声明外 0 个**。
  附记：`git diff --check` 在这个共享检出里会对 `crates/nekowriter-store/src/entity/*` 与
  `crates/nekowriter-agent/src/prompt.rs` 报 CRLF 警告，但 `git diff --numstat HEAD` 对它们
  是 0 行——工作区内容与 HEAD 一致，只是换行符形态，与本批无关（仓库里没有 `.gitattributes`）。
- 门禁（2026-09-16）：`cargo fmt --all -- --check` 通过、`git diff --check` 通过；逐包
  `cargo clippy --all-targets` 六个 crate 全部 **0 warning / 0 error**；
  `cargo clippy --workspace --all-targets` **0 warning / 0 error**；`cargo test --workspace`
  **261 通过 / 0 失败**（agent 44、app 67 + 集成 2、core 38、prompt-lab 2、store 96、xtask 12）；
  `pnpm lint` **0 问题**、`pnpm test` **10 通过 / 0 失败**。
- 本批文件清单：`crates/nekowriter-app/src/main.rs`（2,996 → 2,972 行）、新建
  `crates/nekowriter-app/src/http/state.rs`（42 行）、`crates/nekowriter-app/src/http/mod.rs`（+2 行）。
  未提交：同 P1.1～P1.3；**下一批的基准状态是 `/tmp/nw-split-state/P1.4`**（84 个 Rust 源文件）。

### P1.9 之后 `main.rs` 剩下的区域（现行 1,295 行）

| 行区间 | 内容 | 去向 | 批次 |
| --- | --- | --- | --- |
| 1–28 | `mod` 声明、imports 与 4 条 `pub(crate) use` 再导出（25–28） | 留（crate 根的接线） | — |
| 30–115 | `#[tokio::main] async fn main()` | 留：二进制入口必须在 crate 根 | — |
| 117–1295 | `#[cfg(test)] mod tests`：**30 个 `fn`**（含 harness）共 1,179 行 | 按「P1.10 的测试归属」分散，共享 harness 留 `http/tests.rs` | P1.10（已执行） |

### Phase 1 终态（P1.10 收尾，现行行数）

| 文件 | 行数 | 内容 |
| --- | ---: | --- |
| `app/src/main.rs` | **115** | 13 条 `mod` 声明、导入表、4 条 `pub(crate) use`、`fn main()`；定义数 1 |
| `app/src/startup.rs` | 414 | 启动链 + 2 个测试 |
| `app/src/http/mod.rs` | 343 | `router`、`api_not_found`、`shutdown_service`、子模块声明 + `#[cfg(test)] pub(crate) mod tests;` |
| `app/src/http/dto.rs` | 581 | 39 个 DTO + 4 个形状契约测试 |
| `app/src/http/error.rs` | 682 | `ApiError` 与错误信封 + 3 个测试 |
| `app/src/http/state.rs` | 42 | `AppState`、`EventStreamState` |
| `app/src/http/gate.rs` | 141 | 写信任边界 + 2 个测试 |
| `app/src/http/static_files.rs` | 217 | 静态资源与入口 HTML + 3 个测试（含私有 helper `content_type`） |
| `app/src/http/tests.rs` | 352 | HTTP 测试装置 + 4 个跨域测试（只在 `cfg(test)` 下编译） |
| `app/src/http/handlers/mod.rs` | 106 | 两个跨域 helper（`ensure_model_ready`、`enqueue_queued_writing_run`） |
| `app/src/http/handlers/system.rs` | 415 | 系统面 7 个 handler + 3 个测试 |
| `app/src/http/handlers/prompts.rs` | 156 | 9 个提示词 handler |
| `app/src/http/handlers/projects.rs` | 193 | 项目/版本/导出/项目设置 11 个 handler + `content_disposition` |
| `app/src/http/handlers/chapters.rs` | 381 | 章节/正文 22 个 handler |
| `app/src/http/handlers/preparation.rs` | 211 | 准备域 7 个 handler |
| `app/src/http/handlers/events.rs` | 312 | 事件面 8 项 + 4 个测试 |

**Phase 1 的结论**：`main.rs` 从 **4,240 → 115 行**（十批共搬走 4,125 行），生产代码只剩入口与
接线，定义数 **1**；测试从 1 个 1,179 行的 `mod tests`（25 个测试 + harness）变成 8 个目的模块里
的 8 个 `#[cfg(test)] mod tests` + `http/tests.rs`，测试函数名集合与基线逐字一致；调用方文件
（`store` 的 15 个兄弟模块、crate 根另外 10 个域模块）从 HEAD 到今天一字未改。Phase 1 结束。

`main.rs` 从 4,240 行降到 1,295 行（P1.1–P1.9 九批共搬走 2,945 行，其中 1,179 行是待搬的
测试）。已经出去的是：`ApiError`
（620 行）、39 个 DTO（372 行）、启动链（322 行）、共享状态（42 行）、写信任（75 行）、静态资源
与入口 HTML（53 行）、`http/mod.rs` 的路由表与它的回落/关停端点（338 行）、系统与提示词 handler
（`system.rs` 246 行 + `prompts.rs` 156 行）、项目/版本/导出/项目设置（`projects.rs` 193 行）、
章节域（`chapters.rs` 381 行）、准备域（`preparation.rs` 211 行）与两个跨域 helper
（`http/handlers/mod.rs` 里的 90 行）、事件面（`events.rs` 182 行）。

（P1.10 只剩测试，行号直接从上面这张表读；测试归属表见 §5）

### 2026-09-16 P1.5：路由表、写信任、静态资源与服务关停 → `http/{mod,gate,static_files}.rs`

- 声明：`--from crates/nekowriter-app/src/main.rs`，`--into` 同一文件加
  `crates/nekowriter-app/src/http/mod.rs`、`gate.rs`（新建）、`static_files.rs`（新建）。
- 搬走的东西：`router`（253 行路由表）、`api_not_found`、`shutdown_service` → `http/mod.rs`
  （12 → 321 行）；`require_trusted_write`、`ensure_trusted_write_request`、`is_loopback_host`、
  `allowed_write_origins` 与只被它用的 `VITE_ORIGINS` → `http/gate.rs`（75 行）；
  `serve_client_route`、`is_static_asset_path`、`serve_index_html` → `http/static_files.rs`（53 行）。
  `main.rs` 2,972 → **2,602 行**（删 370 行、增 0 行代码）。
- 为什么 `shutdown_service` 放 `http/mod.rs` 而不是域文件：它是 HTTP 层自己的收尾端点（停止
  接受连接、取消在途请求），不属于任何业务域；它与 `router`、`api_not_found` 同属「把 axum
  应用拼起来」的那一块。域 handler（`health`、设置、章节…）从 P1.6 起才进 `http/handlers/`。
- 原处保留三条再导出，调用点一字未改：`pub(crate) use http::gate::allowed_write_origins;` 与
  `pub(crate) use http::router;`（`main` 里的 `allowed_write_origins(cli.port)`、
  `router(state.clone(), &web_dir)` 不变）。测试模块加一条
  `use crate::http::{gate::is_loopback_host, static_files::serve_index_html};`——这两个名字只被
  测试用，不进 crate 根。
- 本批的接线代价记在案：`http/mod.rs` 的路由表要引用还留在 `main.rs` 的 handler，因此它带一条
  `use crate::{...}`（57 个名字加 5 个域模块）。这条 `use` 属于接线（归一化忽略），路由表本身
  一行没改；P1.6–P1.9 每搬一个域就把它改成 `use crate::http::handlers::<域>::{...}`，于是这批
  的接线预算会逐批缩到零。
- 机械结论分两层：
  1. **单批隔离**（`--base-state /tmp/nw-split-state/P1.4`）：`MOVE-ONLY OK`，`main.rs`
     2,972 → 2,602 行、`http/mod.rs` 12 → 321 行、`gate.rs` 0 → 75 行、`static_files.rs` 0 → 53 行；
     归一化代码行 **2,698 → 2,721**、残留 **0**、放行新增注释 **23 行**。
  2. **整栈核对**（`--base HEAD`，8 个 `--into`）：`MOVE-ONLY OK`，归一化
     **3,890 → 3,936**、残留 **0**、放行新增注释 **46 行**；逐文件 `main.rs +43 −1,681`、
     `http/mod.rs +321`、`gate.rs +75`、`static_files.rs +53`、`startup.rs +322`、
     `http/dto.rs +372`、`http/error.rs +620`、`http/state.rs +42`。
- **独立佐证（不依赖 xtask）**：`git show` + 逐文件归一化 awk + `sort`/`comm`，与工具逐位吻合：
  单批隔离 before/after = **2,698 / 2,721**、未匹配 **0**（after 独有 23 行全是注释）；整栈
  before/after = **3,890 / 3,936**、未匹配 **0**（after 独有 46 行全是注释）。另做一条不丢行的
  原始多重集比对：从 HEAD 的 `main.rs` 删除的行里，新模块找不到的只有 **21 行，全部是导入表
  接线**——18 行是改写过的多行 `use` 块续行（`use std::{...}` 的 `convert::Infallible,` 等 3 行、
  `use axum::{...}` 的 3 行、`nekowriter_core` 名单 11 行、`nekowriter_store` 名单 1 行），
  3 行是整条被删或改写的 `use`（`use tracing::{error, info, warn};`、
  `use nekowriter_core::recycle::RecycleQueryError;`、
  `use nekowriter_store::{CredentialPatch, KnowledgeSpaceInput, NewEmbeddingSettings};`）；
  **代码行 0**。
- 判据缺口：本批没有，工具一行未改；上一批修好的佐证脚本（`use` 行首前缀要跟空白、逐文件独立
  归一）这次直接复用，一次通过。
- 额外机械证据（相对 HEAD，含未跟踪）：**10 个路径** = 被拆文件
  `crates/nekowriter-app/src/main.rs`；新模块 `startup.rs`、
  `http/{mod,dto,error,state,gate,static_files}.rs`；工具 `crates/xtask/src/main.rs`；文档
  `docs/08-giant-file-split-plan.md`。**声明外 0 个**：`main.rs` 里各域 handler、crate 根另外
  10 个域模块与 store 的 15 个兄弟模块从 HEAD 以来一字未改。
- 门禁（2026-09-16）：`cargo fmt --all -- --check` 通过、`git diff --check` 通过；逐包
  `cargo clippy --all-targets` 六个 crate 全部 **0 warning / 0 error**；
  `cargo clippy --workspace --all-targets` **0 warning / 0 error**；`cargo test --workspace`
  **261 通过 / 0 失败**（agent 44、app 67 + 集成 2、core 38、prompt-lab 2、store 96、xtask 12）；
  `pnpm lint` **0 问题**、`pnpm test` **10 通过 / 0 失败**。CRLF 警告仍只出现在未改动的
  `store/src/entity/*` 与 `agent/src/prompt.rs`（`git diff --numstat HEAD` 对它们为 0 行）。
- 本批文件清单：`crates/nekowriter-app/src/main.rs`（2,972 → 2,602 行）、新建
  `crates/nekowriter-app/src/http/gate.rs`（75 行）与 `static_files.rs`（53 行）、
  `crates/nekowriter-app/src/http/mod.rs`（12 → 321 行）。
  未提交：同 P1.1～P1.4；**下一批的基准状态是 `/tmp/nw-split-state/P1.5`**（86 个 Rust 源文件）。

### 2026-09-16 P1.6：系统与提示词 handler → `http/handlers/{system,prompts}.rs`

- 声明：`--from crates/nekowriter-app/src/main.rs` **与** `--from …/http/mod.rs`（见下），`--into`
  两个源文件加新建的 `http/handlers/mod.rs`、`http/handlers/system.rs`、`http/handlers/prompts.rs`。
- 搬走的东西（与 §5 边界表逐条一致，无需调整之处）：
  - `http/handlers/system.rs`（246 行）：`health` `get_capabilities` `get_system_settings`
    `save_system_settings` `system_settings_response` `test_model_connection` `persist_connection_test`；
  - `http/handlers/prompts.rs`（156 行）：`get_system_prompts` `get_project_prompts`
    `save_prompt_override_command` `reset_prompt_override_command` `save_system_prompt_override`
    `reset_system_prompt_override` `save_project_prompt_override` `reset_project_prompt_override`
    `preview_prompt`；
  - `http/handlers/mod.rs`（13 行）：子模块声明与这一层的模块文档（两个跨域 helper 到 P1.8 才来）。
  `main.rs` 2,602 → **2,233 行**（删 369 行、增 0 行代码）；无任何再导出：这批搬走的 handler
  只被 `http::router` 引用，`main.rs` 与测试模块都不引用它们。
- 可见性：两个文件里的 handler 都是 `pub(crate)`（`http::router` 跨模块挂路由）；
  `system_settings_response`、`persist_connection_test`、`save_prompt_override_command`、
  `reset_prompt_override_command` 只被本模块使用，保持私有。
- 接线：`http/mod.rs` 里那条 `use crate::{...}` 缩到 45 个名字，新增两行
  `use crate::http::handlers::{prompts::…, system::…}`；路由表的每个 `.route(...)` 一字未改。
  `main.rs` 侧删掉 6 组不再使用的导入（`nekowriter_agent` 整组、`utc_now_millis`、5 个
  `nekowriter_store` 名字、`SystemOperationResult`、8 个 DTO、`thinking_probe_level`）——都是
  `use` 行，归一化忽略。
- 机械结论分两层：
  1. **单批隔离**（`--base-state /tmp/nw-split-state/P1.5`）：`MOVE-ONLY OK`，归一化代码行
     **2,622 → 2,646**、残留 **0**、放行新增注释 **24 行**；`main.rs` 2,602 → 2,233 行、
     `http/mod.rs` 321 → 326 行、`handlers/mod.rs` 0 → 13、`handlers/system.rs` 0 → 246、
     `handlers/prompts.rs` 0 → 156。
  2. **整栈核对**（`--base HEAD`，11 个 `--into`）：`MOVE-ONLY OK`，归一化 **3,890 → 3,960**、
     残留 **0**、放行新增注释 **70 行**；逐文件 `main.rs +40 −2,047`、`startup.rs +322`、
     `http/mod.rs +326`、`http/dto.rs +372`、`http/error.rs +620`、`http/state.rs +42`、
     `http/gate.rs +75`、`http/static_files.rs +53`、`handlers/mod.rs +13`、`handlers/system.rs +246`、
     `handlers/prompts.rs +156`。
- **判据用法被本批撑出两处修改（工具代码不变，改的是用法与我的佐证脚本）**：
  1. 本批同时改动一个「早前批次已经搬出去的文件」（`http/mod.rs` 多了一行子模块声明与两行
     handler 导入）。第一次跑 `--from main.rs` 就报 260 行残留——不是丢代码，而是 `--from` 决定
     基准集合，只写 `main.rs` 时 `http/mod.rs` 的**既有** 321 行（路由表）全被算成本批新增。
     正确用法：把这一批**包含被搬代码的基准文件**全部列进 `--from`（本批是 `main.rs` +
     `http/mod.rs`），于是 2,622 → 2,646、残留 0。这条已写进 §3。
  2. 同一次还把 `http/mod.rs` 里一句三行的既有文档注释改写了——归一化按行比对，改一行就等于
     删一行，残留 3 行。改成「原三行原样保留 + 新增一行说明」后归零：**新增注释放行，改动既有
     注释算残留**，这也是 P1.1 就定下的规则，这次被真实场景撞上。
- **独立佐证（不依赖 xtask）**：
  - 归一化复刻（`git show` + 逐文件归一 + `sort`/`comm`）与工具逐位吻合：单批 before/after =
    **2,622 / 2,646**、未匹配 **0**、after 独有 24 行全是注释；整栈 = **3,890 / 3,960**、
    未匹配 **0**、after 独有 70 行全是注释。
  - 不丢行的原始多重集比对（只 trim、只丢空行，不丢任何代码行；`comm` 保留重复计数，逐类计数见
    下。归类与工具无关，是行首关键字加「剥掉可见性前缀后是否在另一侧出现」两条机械规则）。
    单批：删除侧 36 行 = **导入接线 24 + 可见性前缀 12 + 代码 0**；新增侧 82 行 =
    **接线（`use`/`mod`/`impl`）46 + 可见性前缀 18 + 注释/文档头 18 + 代码 0**。那 12 行删除侧
    全是搬走的 handler 签名（`async fn health(` 等）——新位置多了 `pub(crate)` 前缀，不是丢代码。
    整栈（HEAD 的 `main.rs` → 当前 11 个文件）：删除侧 232 行 = **导入接线 25 + 可见性前缀 207 +
    代码 0**；新增侧 423 行 = **接线 146 + 可见性前缀 207 + 注释/文档头 70 + 代码 0**。两个方向的
    代码行都是 0。（这一栏用的是原始行多重集，计数自然大于工具那栏的归一化计数与「放行注释」
    数；两者量的是不同的东西，不必相等。）
- 额外机械证据（相对 HEAD，含未跟踪）：**13 个路径** = 被拆文件 `app/src/main.rs`；新模块
  `startup.rs`、`http/{mod,dto,error,state,gate,static_files}.rs`、
  `http/handlers/{mod,system,prompts}.rs`；工具 `crates/xtask/src/main.rs`；文档 `docs/08`。
  **声明外 0 个**：`main.rs` 里 P1.7–P1.9 的 handler、crate 根另外 10 个域模块、store 的 15 个
  兄弟模块从 HEAD 以来一字未改。
- 门禁（2026-09-16）：`cargo fmt --all -- --check` 通过、`git diff --check` 通过；逐包
  `cargo clippy --all-targets` 六个 crate 全部 **0 warning / 0 error**；
  `cargo clippy --workspace --all-targets` **0 warning / 0 error**；`cargo test --workspace`
  **261 通过 / 0 失败**（agent 44、app 67 + 集成 2、core 38、prompt-lab 2、store 96、xtask 12）；
  `pnpm lint` **0 问题**、`pnpm test` **10 通过 / 0 失败**。CRLF 警告仍只出现在未改动的
  `store/src/entity/*` 与 `agent/src/prompt.rs`（`numstat` 为 0 行）。
- 本批文件清单：`crates/nekowriter-app/src/main.rs`（2,602 → 2,233 行）、`http/mod.rs`
  （321 → 326 行）、新建 `http/handlers/mod.rs`（13 行）、`http/handlers/system.rs`（246 行）、
  `http/handlers/prompts.rs`（156 行）。
  未提交：同 P1.1～P1.5；**下一批的基准状态是 `/tmp/nw-split-state/P1.6`**（89 个 Rust 源文件）。

### 2026-09-16 P1.7：项目、版本、导出与项目设置 → `http/handlers/projects.rs`

- 声明：`--from` 三个基准文件（`main.rs`、`http/mod.rs`、`http/handlers/mod.rs`——后两个本批也会
  被改一行接线，按 P1.6 的教训全部列入），`--into` 这四个加新建的
  `http/handlers/projects.rs`。
- 搬走的东西（与 §5 边界表逐条一致）：新建 `http/handlers/projects.rs`（193 行）=
  `list_projects` `get_project` `list_versions` `get_version_snapshot`、`export_version`
  （连两行文档）与它的私有 helper `content_disposition`、`create_project` `create_version`
  `select_version` `copy_version` `get_project_settings`、`save_project_settings`（连两行文档）。
  `main.rs` 2,233 → **2,065 行**（删 168 行、增 0 行代码）；`http/mod.rs` 326 → 330 行（一条
  `use crate::http::handlers::projects::{...}`，11 个名字从那条 `use crate::{...}` 里搬走）；
  `http/handlers/mod.rs` 13 → 15 行（`pub(crate) mod projects;` 加一行模块文档）。
  两处再导出都不需要：这些 handler 只被 `http::router` 引用。
- 可见性：11 个 handler 都 `pub(crate)`；`content_disposition` 只被 `export_version` 用，保持
  私有。测试模块只有三个名字原来从 crate 根取，现在下沉到测试自己的导入表：
  `axum::http::{HeaderValue, header}`（`static_dir_*` 与 `no_cache_*` 两个测试用）与
  `nekowriter_core::NewProject`（`event_cursor_endpoint_*` 用）。
- **非连续行区间的边界怎么定（本批的新东西）**：先 `rg -n '^(pub\(crate\) )?(async )?fn'` 拿到
  全部项的行号，再按「一个项 = 它的文档注释/首个属性 到 闭合大括号 加其后那个空行」切。结果是
  **三段**：`155–181`（项目与版本查询四项）、`192–235`（`export_version` 连文档 +
  `content_disposition`）、`977–1068`（创建/版本切换/设置六项）。§5 表里写的「四段
  （977–1036、1037–1068）」是把相邻的两段写成了两行，实际是同一段；表已按实际改为三段。
  省去的 `183–191`（`get_event_cursor`，属 P1.9）与 `236–976`（章节/事件/准备域）都留在原地。
- **「没有一行因行号漂移而漏搬/错搬」的机械依据（三条，都不靠回读确认）**：
  1. **区间内容 ⇄ 落地文件正文逐行相等**：把 P1.6 快照里那三段按行号取出（160 行），剥掉注释
     行、`use` 段与 `pub(crate) ` 前缀后得 142 行；`projects.rs` 正文同法处理后也是 142 行，
     `comm` 两个方向都是 **0**。即「取出来的行全部落地」且「落地文件里没有一行不在那三段里」
     —— 既没漏、也没错搬。
  2. **定义名字集合的差**：`main.rs` 相对快照少掉的定义恰好是 12 个
     （`list_projects` `get_project` `list_versions` `get_version_snapshot` `export_version`
     `content_disposition` `create_project` `create_version` `select_version` `copy_version`
     `get_project_settings` `save_project_settings`），`projects.rs` 里的定义数也是 **12**；
     `main.rs` 新出现的定义名 **0 个**。
  3. **未被取走的行一动未动**：`main.rs` 相对快照丢失的非 `use` 行（多重集差）全部能在
     `projects.rs` 正文里找到，找不到的 **0 行**；`main.rs` 新增的非 `use` 行 **0 行**。
- 机械结论分两层：
  1. **单批隔离**（`--base-state /tmp/nw-split-state/P1.6`）：`MOVE-ONLY OK`，`main.rs`
     2,233 → 2,065 行、`http/mod.rs` 326 → 330 行、`handlers/mod.rs` 13 → 15 行、`projects.rs`
     0 → 193 行；归一化代码行 **2,301 → 2,313**、残留 **0**、放行新增注释 **12 行**。
  2. **整栈核对**（`--base HEAD`，12 个 `--into`）：`MOVE-ONLY OK`，归一化 **3,890 → 3,972**、
     残留 **0**、放行新增注释 **82 行**；逐文件 `main.rs +117 −2,292`、`startup.rs +322`、
     `http/mod.rs +330`、`dto.rs +372`、`error.rs +620`、`state.rs +42`、`gate.rs +75`、
     `static_files.rs +53`、`handlers/mod.rs +15`、`handlers/system.rs +246`、
     `handlers/prompts.rs +156`、`handlers/projects.rs +193`。
- 判据使用上又撞上 P1.6 写进 §3 的第二条硬话：第一次跑报 **2 行残留**，就是我把
  `http/mod.rs` 与 `handlers/mod.rs` 各一句**既有**模块文档改写了（换成「P1.6 …；P1.7 …」）。
  改成「旧行一字不改 + 追加新行」后残留 0。工具一行未改。
- **独立佐证（不依赖 xtask）**：归一化复刻（`git show` + 逐文件归一 + `sort`/`comm`）与工具逐位
  吻合：单批 **2,301 / 2,313**、残留 0、after 独有 12 行全是注释；整栈 **3,890 / 3,972**、
  残留 0、after 独有 82 行全是注释。不丢行的原始多重集逐类计数（只 trim、只丢空行）：
  单批删除侧 32 行 = **导入/模块接线 21 + 可见性前缀 11 + 代码 0**；单批新增侧 60 行 =
  **接线 37 + 可见性前缀 13 + 注释/文档头 10 + 代码 0**。整栈删除侧 244 行 = **接线 26 +
  可见性前缀 218 + 代码 0**；整栈新增侧 463 行 = **接线 163 + 可见性前缀 218 + 注释/文档头 82 +
  代码 0**。两个方向的代码行都是 0。
- 额外机械证据（相对 HEAD，含未跟踪）：**14 个路径** = 被拆文件 `app/src/main.rs`；新模块
  `startup.rs`、`http/{mod,dto,error,state,gate,static_files}.rs`、
  `http/handlers/{mod,system,prompts,projects}.rs`；工具 `crates/xtask/src/main.rs`；文档 `docs/08`。
  **声明外 0 个**：`main.rs` 里 P1.8–P1.9 的 handler、crate 根另外 10 个域模块、store 的 15 个
  兄弟模块从 HEAD 以来一字未改。
- 门禁（2026-09-16）：`cargo fmt --all -- --check` 通过、`git diff --check` 通过；逐包
  `cargo clippy --all-targets` 六个 crate 全部 **0 warning / 0 error**；
  `cargo clippy --workspace --all-targets` **0 warning / 0 error**；`cargo test --workspace`
  **261 通过 / 0 失败**（agent 44、app 67 + 集成 2、core 38、prompt-lab 2、store 96、xtask 12）；
  `pnpm lint` **0 问题**、`pnpm test` **10 通过 / 0 失败**。CRLF 警告仍只出现在未改动的
  `store/src/entity/*` 与 `agent/src/prompt.rs`（`numstat` 为 0 行）。
- 本批文件清单：`crates/nekowriter-app/src/main.rs`（2,233 → 2,065 行）、`http/mod.rs`
  （326 → 330 行）、`http/handlers/mod.rs`（13 → 15 行）、新建 `http/handlers/projects.rs`
  （193 行）。
  未提交：同 P1.1～P1.6；**下一批的基准状态是 `/tmp/nw-split-state/P1.7`**（90 个 Rust 源文件）。
  P1.8 是最大的一批（22 个章节 handler），且 `ensure_model_ready` 与
  `enqueue_queued_writing_run` 要从 `main.rs` 搬进 `http/handlers/mod.rs`。

### 2026-09-16 P1.8：章节域、准备域与两个跨域 helper → `handlers/{chapters,preparation,mod}.rs`

- 声明：`--from` 三个基准文件（`main.rs`、`http/mod.rs`、`http/handlers/mod.rs`），`--into` 这四个
  加新建的 `handlers/chapters.rs`、`handlers/preparation.rs`。
- 搬走的东西（与 §5 边界表逐条一致）：
  - `handlers/chapters.rs`（381 行）：从 `list_chapters` 到 `submit_chapter_final_review` 的 22 个
    handler（`main.rs` 快照的 158–507，一整段连续区间）。
  - `handlers/preparation.rs`（211 行）：两段——`get_preparation` `save_novel_brief`
    `start_preparation`（647–695）与 `submit_preparation_artifacts` `decide_review` `delegate_review`
    `cancel_review_delegation`（724–857），中间隔着第一个跨域 helper。
  - `handlers/mod.rs`（15 → 104 行）：两个跨域 helper `ensure_model_ready`（697–723）与
    `enqueue_queued_writing_run`（859–897），共 90 行。
  `main.rs` 2,065 → **1,449 行**（删 616 行、增 0 行代码）；`http/mod.rs` 330 → 336 行。
- **共享模块依赖的归属与可见性（本批第一次真出现，现场定下的结论）**：两个 helper 都放在
  `http/handlers/mod.rs`，可见性 `pub(crate)`，两个域文件用
  `use crate::http::handlers::{ensure_model_ready, enqueue_queued_writing_run};` 取。三条依据：
  ① 调用点是交错的——`ensure_model_ready` 被章节域的 `start_chapter_outline`/`continue_writing_run`
  与准备域的 `start_preparation`/`delegate_review` 各调两次；`enqueue_queued_writing_run` 被章节
  域调 5 次、准备域调 1 次。放任一域文件都会让另一域反向依赖。② 它们自身的依赖很窄：只用到
  `AppState`、`nekowriter_core` 与 `nekowriter_store`，所以放到父模块不需要任何额外接线（这正
  是“能不能只靠 `pub(crate) use` 接回”的答案：不需要 `use`，子模块直接 `super::` 取）。
  ③ 不需要反向导出到 crate 根：调用方都在 `http` 子树内。结论已写进 `handlers/mod.rs` 的模块
  文档与 §5 的边界表。
- 可见性与导入下沉：37 个搬走的项全部 `pub(crate)`（`http::router` 挂路由）；`content_disposition`
  式的私有项本批没有。`main.rs` 的导入表因此大幅变窄：删掉整组 `nekowriter_core`、整组
  `crate::http::dto`、整组 `crate::model_settings`、`serde_json::json`、`uuid::Uuid`、
  `StoreError`、`StatusCode`、`Path as AxumPath`；其中只被测试用的名字（`StatusCode`、
  `StoreError`、`AnswerQuestionRequest`、`DecidePreparationReviewRequest`、
  `StartPreparationRequest`、`serde_json::{Value, json}`、`uuid::Uuid`）下沉到测试模块的导入表
  （第 4 次用同一个手法：crate 根只留生产代码真正用到的名字）。`http/mod.rs` 里那条
  `use crate::{...}` 缩到一行 7 个名字（`dashboard` `get_event_cursor` `knowledge` `manuscript`
  `recycle` `stream_events` `templates`），路由表一字未改。
- **判据缺口（第三处，这次在工具里）**：第一次跑报 **5 行残留**，全部来自同一个签名：
  `ensure_model_ready` 在基准里是一行（`async fn ensure_model_ready(state: &AppState, project_id:
  ProjectId) -> Result<(), ApiError> {`，94 字符），加上 `pub(crate) ` 前缀后是 105 字符，
  **rustfmt 必然把它折成四行**。这不是行为变化，而是排版。修法：归一化新增一条规则——把函数
  签名（`fn` / `async fn` / `unsafe fn` / `const fn` 行到它那个 `{` 或 `;`）合并成一行，并去掉
  折行留下的 `( ` / ` )` / `, ` / `,)` 空白；于是「同一个签名、不同折行」算同一行，而参数名、
  类型、返回值改了仍然报残留（新增两条用例：`a_wrapped_signature_is_layout_not_code`、
  `a_changed_signature_parameter_still_shows_up`；`cargo test -p xtask` **14 通过 / 0 失败**）。
  **注意：这条规则改变了归一化行数的可比性**——本批的「搬迁前」在旧规则下是 3,890、在新规则
  下是 3,650（差在基准文件里那些被折叠的多行签名）；跨工具版本不能直接比数字，同一批的
  前/后与残留才是判据。
- 机械结论分两层：
  1. **单批隔离**（`--base-state /tmp/nw-split-state/P1.7`）：`MOVE-ONLY OK`，`main.rs`
     2,065 → 1,449 行、`http/mod.rs` 330 → 336 行、`handlers/mod.rs` 15 → 104 行、
     `chapters.rs` 0 → 381、`preparation.rs` 0 → 211；归一化代码行 **2,021 → 2,043**、
     残留 **0**、放行新增注释 **22 行**。
  2. **整栈核对**（`--base HEAD`，14 个 `--into`）：`MOVE-ONLY OK`，归一化 **3,650 → 3,754**、
     残留 **0**、放行新增注释 **104 行**；逐文件 `main.rs +98 −2,889`、`startup.rs +322`、
     `http/mod.rs +336`、`dto.rs +372`、`error.rs +620`、`state.rs +42`、`gate.rs +75`、
     `static_files.rs +53`、`handlers/mod.rs +104`、`system.rs +246`、`prompts.rs +156`、
     `projects.rs +193`、`chapters.rs +381`、`preparation.rs +211`。
- **独立佐证（不依赖 xtask）**：归一化复刻（同样实现了签名合并规则）与工具逐位吻合：单批
  **2,021 / 2,043**、残留 0、after 独有 22 行全是注释；整栈 **3,650 / 3,754**、残留 0、
  after 独有 104 行全是注释。不丢行的原始多重集逐类计数（只 trim、只丢空行；分类规则：
  `use`/`mod`/`impl` 行算接线、签名块里「合并后能对上另一侧」的行算签名折行、剥掉可见性前缀
  后能在另一侧找到的行算可见性前缀、`//` 开头算注释）：单批删除侧 55 行 = **接线 24 +
  签名折行 31 + 代码 0**，单批新增侧 120 行 = **接线 64 + 签名折行 34 + 可见性前缀 5 +
  注释 17 + 代码 0**；整栈删除侧 275 行 = **接线 26 + 签名折行 63 + 可见性前缀 186 + 代码 0**，
  整栈新增侧 559 行 = **接线 203 + 签名折行 66 + 可见性前缀 186 + 注释 104 + 代码 0**。
  两个方向的代码行都是 0。
- **定义名字集合的机械核对**：`main.rs` 相对 P1.7 快照少掉的定义恰好 31 个（22 章节 + 7 准备
  + 2 跨域 helper），三个新文件的定义数也是 **31**（22 + 7 + 2）；`main.rs` **新增定义 0 个**；
  两边集合的差在两个方向上都是 **0**。`main.rs` 的定义数 40 → 9（`main` 与事件面八项，全部
  属 P1.9/P1.10 的活）。
- 额外机械证据（相对 HEAD，含未跟踪）：**16 个路径** = 被拆文件 `app/src/main.rs`；新模块
  `startup.rs`、`http/{mod,dto,error,state,gate,static_files}.rs`、
  `http/handlers/{mod,system,prompts,projects,chapters,preparation}.rs`；工具
  `crates/xtask/src/main.rs`；文档 `docs/08`。**声明外 0 个**：`main.rs` 里 P1.9 的事件面、
  crate 根另外 10 个域模块、store 的 15 个兄弟模块从 HEAD 以来一字未改。
- 门禁（2026-09-16）：`cargo fmt --all -- --check` 通过、`git diff --check` 通过；逐包
  `cargo clippy --all-targets` 六个 crate 全部 **0 warning / 0 error**；
  `cargo clippy --workspace --all-targets` **0 warning / 0 error**；`cargo test --workspace`
  **263 通过 / 0 失败**（agent 44、app 67 + 集成 2、core 38、prompt-lab 2、store 96、xtask 14）；
  `pnpm lint` **0 问题**、`pnpm test` **10 通过 / 0 失败**。CRLF 警告仍只出现在未改动的
  `store/src/entity/*` 与 `agent/src/prompt.rs`（`numstat` 为 0 行）。
- 本批文件清单：`crates/nekowriter-app/src/main.rs`（2,065 → 1,449 行）、`http/mod.rs`
  （330 → 336 行）、`http/handlers/mod.rs`（15 → 104 行）、新建 `http/handlers/chapters.rs`
  （381 行）、`http/handlers/preparation.rs`（211 行）；工具 `crates/xtask/src/main.rs`（签名
  合并规则 + 2 条用例）。
  未提交：同 P1.1～P1.7；**下一批的基准状态是 `/tmp/nw-split-state/P1.8`**（92 个 Rust 源文件）。
  下一批 P1.9 只剩两段：`get_event_cursor`（129–137）与事件面七项（139–275）。

### 2026-09-16 P1.9：事件面 → `http/handlers/events.rs`（Phase 1 生产代码收尾）

- 声明：`--from` 三个基准文件（`main.rs`、`http/mod.rs`、`http/handlers/mod.rs`），`--into` 这四个
  加新建的 `handlers/events.rs`。
- 搬走的东西（与 §5 边界表逐条一致）：新建 `handlers/events.rs`（182 行）——`get_event_cursor`
  （连它那行文档）、`stream_events`、`next_sse_event`、`to_sse_event`、
  `to_preview_delta_sse_event`、`event_cursor`、`stream_reset_required`、`stream_reset_event`，
  即 `main.rs` 快照的 129–275 两段。`main.rs` 1,449 → **1,295 行**（删 154、增 0 行代码）；
  `http/mod.rs` 336 → 338 行（新增一行从 `events` 取两个 handler 的 `use`）；
  `handlers/mod.rs` 104 → 106 行（`pub(crate) mod events;` 加一行模块文档）。路由表一字未改。
- 可见性：两个 handler `pub(crate)`（`http::router` 挂 `/events` 与 `/events/stream`）；
  `next_sse_event`、`to_sse_event`、`to_preview_delta_sse_event`、`stream_reset_event` 私有；
  **`event_cursor` 与 `stream_reset_required` 例外**——`main.rs` 里尚未搬走的两个测试
  （`event_cursor_prefers_last_event_id_and_rejects_invalid_values`、
  `stream_reset_required` 的四条断言所在的测试）仍在调用它们，所以暂时 `pub(crate)`，并在
  测试模块里加了
  `use crate::http::handlers::events::{event_cursor, stream_reset_required};`。**P1.10 把那两个
  测试搬进 `events.rs` 之后，这两个名字应当退回私有**（已写进 `events.rs` 的模块文档与 §5）。
- 导入表大清扫（全部是 `use` 行，归一化忽略）：`main.rs` 删掉整块 `use axum::{...}`（生产区只
  剩下全限定的 `axum::serve`）、`futures_util::stream`、`nekowriter_store::{EventStreamBounds,
  PersistedEvent}`（保留 `InterruptionSource`）、`crate::http::dto::{EventCursorResponse,
  EventsQuery}`、std 里的 `VecDeque` / `Infallible`，并把再导出改回
  `pub(crate) use http::state::AppState;`（`EventStreamState` 已无使用者，删掉）。只被测试用的
  4 类名字（`Duration`、`IntoResponse`、`HeaderMap`、`EventStreamBounds`）下沉到测试模块的
  导入表（第 5 次用同一手法）。
- **Phase 1 生产代码的验收点（用户要求的那条）**：搬完之后 `http/mod.rs` 里那条
  `use crate::{...}` 只剩 **五个域模块**——`use crate::{dashboard, knowledge, manuscript,
  recycle, templates};`。逐个核对用途：`dashboard` 1 次、`knowledge` 19 次、`manuscript` 4 次、
  `recycle` 8 次、`templates` 6 次，**全是 `router` 里带前缀的调用**（如
  `get(dashboard::get_dashboard)`、`get(recycle::get_recycle_bin)`），没有任何一个名字可以再删；
  所有 handler 名字都改从 `crate::http::handlers::<域>` 取（六条 `use`，见
  `http/mod.rs:28–52`）。同时 `main.rs` 的生产区只剩导入表 + 4 条再导出 + `fn main()`，
  **定义数 1**。这两个条件已写进 §6 作为 Phase 1 的验收点。
- 机械结论分两层：
  1. **单批隔离**（`--base-state /tmp/nw-split-state/P1.8`）：`MOVE-ONLY OK`，`main.rs`
     1,449 → 1,295 行、`http/mod.rs` 336 → 338 行、`handlers/mod.rs` 104 → 106 行、
     `events.rs` 0 → 182 行；归一化代码行 **1,634 → 1,647**、残留 **0**、放行新增注释 **13 行**。
  2. **整栈核对**（`--base HEAD`，15 个 `--into`）：`MOVE-ONLY OK`，归一化 **3,650 → 3,767**、
     残留 **0**、放行新增注释 **117 行**；逐文件 `main.rs +32 −2,977`、`startup.rs +322`、
     `http/mod.rs +338`、`dto.rs +372`、`error.rs +620`、`state.rs +42`、`gate.rs +75`、
     `static_files.rs +53`、`handlers/mod.rs +106`、`system.rs +246`、`prompts.rs +156`、
     `projects.rs +193`、`chapters.rs +381`、`preparation.rs +211`、`events.rs +182`。
- 判据：本批没有新缺口（工具一行未改，上一批的签名合并规则直接跑过）；既有注释只做「原样
  保留 + 追加」，因此也没有注释类残留。
- **独立佐证（不依赖 xtask）**：归一化复刻与工具逐位吻合：单批 **1,634 / 1,647**、残留 0、
  after 独有 13 行全是注释；整栈 **3,650 / 3,767**、残留 0、after 独有 117 行全是注释。
  不丢行的原始多重集逐类计数：单批删除侧 11 行 = **接线 7 + 签名折行 4 + 代码 0**，新增侧
  41 行 = **接线 21 + 签名折行 7 + 可见性前缀 2 + 注释 11 + 代码 0**；整栈删除侧 279 行 =
  **接线 26 + 签名折行 67 + 可见性前缀 186 + 代码 0**，新增侧 593 行 = **接线 217 +
  签名折行 73 + 可见性前缀 186 + 注释 117 + 代码 0**。两个方向的代码行都是 0。
- **定义名字集合的机械核对**：`main.rs` 相对 P1.8 快照少掉的定义恰好 **8** 个
  （`get_event_cursor` `stream_events` `next_sse_event` `to_sse_event`
  `to_preview_delta_sse_event` `event_cursor` `stream_reset_required` `stream_reset_event`），
  `events.rs` 里的定义数也是 **8**；`main.rs` **新增定义 0 个**；两边集合的差两个方向都是 **0**。
  `main.rs` 的定义数 9 → **1**（只剩 `main`）。
- 额外机械证据（相对 HEAD，含未跟踪）：**17 个路径** = 被拆文件 `app/src/main.rs`；新模块
  `startup.rs`、`http/{mod,dto,error,state,gate,static_files}.rs`、
  `http/handlers/{mod,system,prompts,projects,chapters,preparation,events}.rs`；工具
  `crates/xtask/src/main.rs`；文档 `docs/08`。**声明外 0 个**：crate 根另外 10 个域模块、
  store 的 15 个兄弟模块从 HEAD 以来一字未改。
- 门禁（2026-09-16）：`cargo fmt --all -- --check` 通过、`git diff --check` 通过；逐包
  `cargo clippy --all-targets` 六个 crate 全部 **0 warning / 0 error**；
  `cargo clippy --workspace --all-targets` **0 warning / 0 error**；`cargo test --workspace`
  **263 通过 / 0 失败**（agent 44、app 67 + 集成 2、core 38、prompt-lab 2、store 96、xtask 14）；
  `pnpm lint` **0 问题**、`pnpm test` **10 通过 / 0 失败**。CRLF 警告仍只出现在未改动的
  `store/src/entity/*` 与 `agent/src/prompt.rs`（`numstat` 为 0 行）。
- 本批文件清单：`crates/nekowriter-app/src/main.rs`（1,449 → 1,295 行）、`http/mod.rs`
  （336 → 338 行）、`http/handlers/mod.rs`（104 → 106 行）、新建 `http/handlers/events.rs`
  （182 行）。
  未提交：同 P1.1～P1.8；**下一批的基准状态是 `/tmp/nw-split-state/P1.9`**（93 个 Rust 源文件）。
  下一批 P1.10 只剩测试归属：`main.rs` 的 30 个 `fn`（1,179 行，即 117–1295）按 §5 的表分散，共享 harness
  进 `http/tests.rs`；两个因测试而暂时 `pub(crate)` 的 helper 在那批退回私有。

### 2026-09-16 P1.10：25 个测试分散到 8 个模块（Phase 1 收尾）

- 声明（单批隔离）：`--base-state /tmp/nw-split-state/P1.9`；`--from` 与本批会改的 9 个基准文件
  （`main.rs`、`http/mod.rs`、`startup.rs`、`http/{gate,static_files,dto,error}.rs`、
  `http/handlers/{events,system}.rs`），`--into` 同上再加新建的 `http/tests.rs`；`--allow` 是这
  10 个路径加 `crates/xtask/src/main.rs`（默认 allow 是 `--from ∪ --into`，工具自身的改动要显式列）。
- 搬走的东西（函数名集合与基线逐字一致，25 个一个不多一个不少）：
  - `startup.rs`：`startup_failure_messages_name_the_directory_step_cause_and_way_forward`、
    `default_web_dir_lives_next_to_the_executable`（+92 行）。
  - `http/gate.rs`：`loopback_host_validation_rejects_external_hosts`、
    `write_trust_is_enforced_by_the_router_not_by_handlers`（+66）。
  - `http/static_files.rs`：`index_html_is_revalidated_before_loading_route_chunks`、
    `deep_links_fall_back_to_the_entry_html_without_swallowing_api_errors`、
    `static_dir_serves_client_routes_and_reports_missing_assets_honestly`（+164，含私有 helper
    `content_type`）。
  - `http/handlers/events.rs`：4 个 SSE/游标测试（+130）。
  - `http/dto.rs`：4 个形状契约测试（+209）。
  - `http/handlers/system.rs`：3 个设置/embedding 测试（+169）。
  - `http/error.rs`：3 个错误信封测试（+62）。
  - 新建 `http/tests.rs`（352 行）：harness + 3 个积累命令契约 + `restored_ui_capabilities_...`。
  - `http/mod.rs` +5 行（`#[cfg(test)] pub(crate) mod tests;` 与两行模块文档）；`main.rs`
    1,295 → **115 行**（−1,180），`git diff --numstat` 新增代码 0 行。
- **生产区一字未动的机械依据**：`diff <(head -115 /tmp/nw-split-state/P1.9/…/main.rs) main.rs`
  无输出（115 行逐字相同），且整栈核对里 `main.rs` 的删除侧未匹配行代码为 0。
- 两处对 §5 harness 归属的现场调整（边界表其余各条一字不改）：① `new_accumulation_space` 只有
  `http/tests.rs` 里的 3 个积累测试用，留在该文件但保持私有，不进对外 harness；② `content_type`
  只有 `static_files` 的那条测试用（2 个调用点），跟着它搬进 `http/static_files.rs` 的测试模块当
  私有 helper——做成共享会是一个只有单个消费者的 `pub(crate)` 名字。因此跨模块 harness 就是
  `accumulation_app` / `spawn_app` / `spawn_app_with_web_dir` 三个。P1.9 的两个临时 `pub(crate)`
  （`event_cursor`、`stream_reset_required`）随测试搬回 `events.rs` 后退回私有：
  `rg 'pub\(crate\) fn (event_cursor|stream_reset_required)'` 为 0 命中，clippy 无 `dead_code`。
- 判据口径的第四处缺口（这次在工具里）：**搬测试会把一个 `mod tests` 块变成多个**——目的文件
  各自新建 `#[cfg(test)] mod tests`，包装行（属性行与闭合 `}`）与源文件不成对。修法是 `xtask`
  的归一化多一条：紧邻 `mod` 头的 `#[cfg(...)]` 属性行与「被丢掉的 `mod` 块」的闭合 `}` 一并算
  接线（新增 `brace_delta` 跟踪花括号深度定位那个闭合括号）；配三条用例（一个测试模块拆成两个、
  包装内改一行必须报残留、cfg 属性挂在非 `mod` 项上照旧比对），`cargo test -p xtask`
  **16 通过 / 0 失败**。规则只去掉两侧成对的结构行，所以早前批次只是行数口径变化、结论不变：
  P1.8 → P1.9 的数据在新旧规则下都是残留 0（新 1,633 → 1,646，旧 1,634 → 1,647）。同一份 P1.10
  声明在旧规则下会报 **13 行残留**（7 行 `#[cfg(test)]` + 6 行 `}`），新规则下为 0。
- 机械结论：
  1. 单批隔离：**`MOVE-ONLY OK`**，归一化代码行 **3,028 → 3,040**、残留 **0**、放行新增注释
     **12 行**；逐文件 `main.rs` 1,295 → 115、`http/mod.rs` 338 → 343、`startup.rs` 322 → 414、
     `gate.rs` 75 → 141、`static_files.rs` 53 → 217、`dto.rs` 372 → 581、`error.rs` 620 → 682、
     `handlers/events.rs` 182 → 312、`handlers/system.rs` 246 → 415、`http/tests.rs` 0 → 352。
  2. 整栈核对（`--base HEAD`，16 个 `--into`）：**`MOVE-ONLY OK`**，**3,649 → 3,778**、残留 **0**、
     放行新增注释 **129 行**。
- 独立佐证（不依赖 xtask）：`git show` + 逐文件归一化 awk（同样实现了包装规则）+ `sort`/`comm`
  与工具**逐位吻合**（单批 3,028 / 3,040、残留 0、after 独有 12 行全是注释；整栈 3,649 / 3,778、
  残留 0、after 独有 129 行全是注释）。不丢行的原始多重集逐类计数：删除侧 1,099 行独有（快照
  `main.rs` 独有）、新增侧 1,157 行独有（9 个目的文件独有 + `tests.rs` 全文）；未匹配删除侧 **19**
  = 16 导入接线 + 3 可见性前缀 + **代码 0**；未匹配新增侧 **77** = 40 导入接线 + 14 测试模块包装
  + 6 包装闭合括号 + 12 注释/模块文档头 + 3 可见性前缀 + 2「收回私有」（`event_cursor` /
  `stream_reset_required` 去掉 `pub(crate)` 后签名被 rustfmt 折回一行），**代码 0**。
  佐证脚本自身一处 bug 已修：`pub(crate) ` 前缀是 11 字符，我按 12 写，导致 8 行误报为未匹配。
- 测试归属的机械证据：`cargo test -p nekowriter-app --bins -- --list` 前后都是 **67** 个用例；
  25 个 `tests::<名字>` 的 leaf 名字集合**两个方向差集都是 0**（用例名一个不多一个不少、没有新增
  或删除用例）；新模块路径为 `startup::tests::*`、`http::{dto,error,gate,static_files}::tests::*`、
  `http::handlers::{events,system}::tests::*`、`http::tests::*`。函数体逐字未改由上面两条多重集
  证据保证（归一化残隉 0 + 原始多重集代码行 0），断言一行未改。
- 调用方未动：相对 HEAD（含未跟踪）共 17 个路径 = 本批的 10 个 + P1.1–P1.9 已搬的 7 个模块文件
  + `crates/xtask/src/main.rs`；相对 P1.9 快照只有本批声明的 11 个（10 个 app 文件 + 工具）。
  声明外 **0**——`store` 的 15 个兄弟模块与 crate 根另外 10 个域模块自 HEAD 以来一字未改；
  `nekowriter-prompt-lab` 与 `/tmp/nw-story-run` 未碰，无密钥进入响应或日志。
- 门禁（2026-09-16）：`cargo fmt --all -- --check` 通过、`git diff --check` 通过；逐包
  `cargo clippy --all-targets` 六个 crate 全部 **0 warning / 0 error**；
  `cargo clippy --workspace --all-targets`（强制全量重编译）**0 warning / 0 error**；
  `cargo test --workspace` **265 通过 / 0 失败**（agent 44、app 67 + 集成 2、core 38、
  prompt-lab 2、store 96、xtask 16）；`pnpm lint` **0 问题**、`pnpm test` **10 通过 / 0 失败**。
  CRLF 警告仍只出现在未改动的 `store/src/entity/*` 与 `agent/src/prompt.rs`。
- **Phase 1 结束**：验收点见 §6，终态行数表见上（`main.rs` 4,240 → 115、定义数 1）。**下一阶段
  （Phase 2 的 `store/lib.rs`）的基准快照：`/tmp/nw-split-state/P1.10`（94 个 Rust 源文件）**；
  入口条件是 Phase 1 的十批仍以未提交状态留在工作区（同一检出、同一批次状态目录），P2.1 起用
  `--base-state /tmp/nw-split-state/P1.10` 做单批隔离、用 `--base HEAD` 做整栈核对，并显式给
  `--scope crates`（Phase 2 的搬迁跨 `store` 内多个文件）。
  实际执行时 Phase 2 另写了一份自己的基准快照 **`/tmp/nw-split-state/P2.0`**（94 个文件）：它
  与 P1.10 的**产品源码逐文件相同**，差的只有 `crates/xtask/src/main.rs`（那是 P1.10 之后判据
  加固那一轮改的工具）——两个快照做单批隔离都得同一结论，取哪一个都行。

---

### 2026-09-16 判据加固：从「行池化多重集」改成「按定义项逐项有序比对」（只改 `crates/xtask` 与文档）

上一轮只读审计用探针矩阵证明了旧判据的两处真盲区：① I2 把全部 `--from` 与全部 `--into` 各自
池化成一个多重集再比，于是**行/函数体的置换**（审计在副本里把细纲与正文两个 match 分支的实现
互换）与**跨文件再分配**完全隐形；② P1.10 新加的「`mod` 包装算接线」把 `#[cfg(...)]` 属性行也
丢掉，于是把 `#[cfg(test)]` 改成 `#[cfg(all(test, feature="…"))]`（让测试从编译里消失）也报 OK。
这一轮把两处都堵上，并用 16 条探针 + 全部十批快照回验。

**工具改了什么**（`crates/xtask/src/main.rs`：本批 −181 行 / +834 行，1,480 → 2,133 行；用例
16 → 20 条）

1. **I2 的单位从「行池」改成「定义项」**。`collect_items` 把每个文件切成项与项外自由行；
   `item_residuals` 按 key 分组、组内按**有序内容**配对：内容完全相同的互相抵消（项整体换文件、
   `impl` 一分为二仍算搬迁成功），一侧多出来的整项点名为新增/删除，成对而行多重集不同就逐行列出，
   **行多重集相同但顺序不同**则留下 `~ 行序改变（行多重集相同）`。
2. **cfg 条件参与比对**：`#[cfg(test)]` 算接线（搬测试必然要在新位置补上它）；去掉 `test` 谓词后
   仍有条件的 cfg 做成 `cfg: …` 行进项内容（`all(test, feature = "x")` → `cfg: all(feature="x")`，
   容器上的 cfg 由内部每一项继承）。`feature = "test-mode"` 里的字符串不算 `test` 谓词。
3. **`impl` 与 `mod` 变成透明容器**：头行不参与比对，`impl` 头进内部项的 key，`mod` 的名字不进。
   这取代了上一轮的第四条口径（`#[cfg(...)]` 与闭合 `}` 一起算接线）：结构由容器关系表达，
   不再靠丢行。
4. **花括号深度改成按词法扫**：旧实现朴素地数 `{` / `}`，会被字符串骗到——P1.9 的
   `preview_delta_shares_sse_without_a_persistent_id` 里就有只写了一半括号的字符串，深度失衡后
   后面所有项被吞进同一个项（实测 `main.rs` 只切出 25 个项，实际 39 个）。新的 `Braces` 跳过注释、
   字符串（含 `r#"…"#`）与字符字面量（生命周期 `'a` 不算）。这是行池化判据不会暴露的正确性问题：
   判据按项分组后，项边界必须对。
5. **新增 `--head-state <目录>`**（任意一批都能重跑）与 **`--list-items`**（只列项与项外自由行，
   不做判定）。

**探针矩阵**（全部在 `/tmp` 副本上改；基线 = P1.9 → P1.10 未改动的一对：退出码 0、项 138 / 138、
残留 0）

| 探针 | 改动 | 期望 | 实际 | 退码 | 点名的行（`--list-items` 之外的判定输出） |
|---|---|---|---|---|---|
| A1 | `static_files.rs` `contains('.')` → `contains(':')` | 抓到 | 抓到 | 1 | `- [fn is_static_asset_path(path: &str) -> bool] …contains('.')` / `+ …contains(':')` |
| A2 | `events.rs` `cursor < bounds.minimum_cursor` → `<=` | 抓到 | 抓到 | 1 | `- [fn stream_reset_required(cursor: i64,bounds: EventStreamBounds) -> bool] cursor < …` |
| B1 | `system.rs` 去掉 `\| ModelFailureCode::Timeout` | 抓到 | 抓到 | 1 | `- [async fn persist_connection_test(…)] ModelFailureCode::Connection \| ModelFailureCode::Timeout` |
| C1 | `tests.rs` 项内调换两条 `let`（审计的 C1） | 抓到 | 抓到 | 1 | `~ [async fn accumulation_app(engines: &'static [&'static dyn AccumulationEngine]) -> (TempDir,AppState)] 行序改变（行多重集相同）` |
| C2 | `handlers/mod.rs` 对调两个匹配分支的实现（审计的 C2） | 抓到 | 抓到 | 1 | `~ [async fn enqueue_queued_writing_run(state: &AppState,workflow: &WorkflowSnapshot) -> Result<(),ApiError>] 行序改变（行多重集相同）` |
| C3 | 把一个函数挪到新文件时漏搬两行 | 抓到 | 抓到 | 1 | 3 行 `- [fn is_static_asset_path(path: &str) -> bool] …` |
| C4 | 把一个函数整体挪到新文件（跨文件再分配） | 放行 | 放行 | 0 | — |
| D1 | 签名折行 + 加 `pub(crate) ` | 放行 | 放行 | 0 | — |
| D2 | 两条 `use` 合成一条折行的 | 放行 | 放行 | 0 | — |
| D3 | 改缩进 | 放行 | 放行 | 0 | — |
| E1 | 删掉一行既有注释 | 抓到 | 抓到 | 1 | `- /// 它挂成一层中间件，而不是由每个 handler 记得调用：新增写端点时不会漏掉这条校验。` |
| E2 | 新增一行注释 | 放行 | 放行 | 0 | 放行新增注释 12 → 13 行 |
| F1 | 签名写成等价的全限定路径（`Response` → `axum::response::Response`） | 抓到 | 抓到 | 1 | 20 行，`- [async fn require_trusted_write(…)] …`（与旧判据一样偏严：等价重写也报） |
| G1 | 容器 cfg `#[cfg(test)]` → `#[cfg(all(test, feature = "audit_probe"))]` | 抓到 | 抓到 | 1 | `+ [fn loopback_host_validation_rejects_external_hosts()] cfg: all(feature="audit_probe")` 等 2 行 |
| H1 | 把一个 `impl` 一分为二 | 放行 | 放行 | 0 | — |
| H2 | 两个 `impl` 的同名方法对调实现 | 抓到 | 抓到 | 1 | 4 行，`- [impl AccumulationEngine for StubAccumulationEngine \| fn descriptor(&self) -> EngineDescriptor] id: "stub-accumulation",` 等 |

C1、C2、G1 正是上一轮审计里「退出码 0（MOVE-ONLY OK）」的三条，现在都在 I2 里被抓到并点名到项；
D1 / D2 / D3 / E2 / H1 / C4 六类合法形状仍然放行。

**十批回验**（新增 `--head-state`，用快照两两重跑；声明由快照差集机械推导，检查器自身的源码只进
`--allow`）

| 跑的批次 | 基准 → 位置 | 定义项（前/后） | 项内行（前/后） | 残留 | 结论 |
|---|---|---|---|---|---|
| P1.1 + P1.2 | HEAD 快照 → P1.2 | 192 / 192 | 3,501 / 3,501 | 0 | MOVE-ONLY OK（放行新增注释 19 行） |
| P1.3 | P1.2 → P1.3 | 136 / 136 | 2,635 / 2,635 | 0 | MOVE-ONLY OK（放行 5 行） |
| P1.4 | P1.3 → P1.4 | 120 / 120 | 2,397 / 2,397 | 0 | MOVE-ONLY OK（放行 7 行） |
| P1.5 | P1.4 → P1.5 | 118 / 118 | 2,375 / 2,375 | 0 | MOVE-ONLY OK（放行 15 行） |
| P1.6 | P1.5 → P1.6 | 110 / 110 | 2,300 / 2,300 | 0 | MOVE-ONLY OK（放行 24 行） |
| P1.7 | P1.6 → P1.7 | 94 / 94 | 2,032 / 2,032 | 0 | MOVE-ONLY OK（放行 12 行） |
| P1.8 | P1.7 → P1.8 | 82 / 82 | 1,927 / 1,927 | 0 | MOVE-ONLY OK（放行 22 行） |
| P1.9 | P1.8 → P1.9 | 53 / 53 | 1,533 / 1,533 | 0 | MOVE-ONLY OK（放行 13 行） |
| P1.10 | P1.9 → P1.10 | 138 / 138 | 2,837 / 2,837 | 0 | MOVE-ONLY OK（放行 12 行） |
| 整栈（十批） | HEAD 快照 → 工作区（16 个 `--into`） | 192 / 192 | 3,501 / 3,501 | 0 | MOVE-ONLY OK（放行新增注释 129 行） |

- **十批全部 `MOVE-ONLY OK`、残留 0，没有需要归类的残留行**：新判据下没有发现任何真行为差异，
  也没有接线 / 可见性 / 注释 / 排版类的残留（逐类计数全为 0）。**结论与各批当时的记录一致**，
  差别只在数字口径（见下）。
- 唯一被新判据抓出来的「改动」是**检查器自己的源码**：把 `crates/xtask/src/main.rs` 放进
  `--from` / `--into` 时，P1.9 → P1.10 会多报 20 行（P1.10 新增的 `fn brace_delta` 11 行 + `fn
  cfg_attributes_are_only_wiring_when_they_wrap_a_module` 9 行）。那是改判据时对工具自己的改动，
  不是产品代码；按 §3 第三句硬话只进 `--allow`。顺带说明：新判据**能**抓到项级的新增与删除。
- **独立收支核对**（不依赖 xtask 的项切分）：对 P1.9 → P1.10 两侧的 19 个文件侧，独立算
  「非空行 − use 语句（含折行）− 容器头与配对括号 − `mod x;` 声明 − 只带 test 的 cfg 行 − 签名
  折行合并掉的行」，与 `--list-items` 报出的「项内 + 项外」逐文件比：**17 个文件侧差 0**，
  `main.rs` 两侧同为 **−4**（那 4 行是 `pub(crate) use` 再导出：朴素计数器不把带可见性的 `use`
  当接线；两侧一致，不影响判定）。即：项分解既不丢行也不重复计行。
- **旧数字的效力**：见本节开头的「读数字前先看口径」。
- 门禁（2026-09-16，本批）：`cargo fmt --all -- --check` 通过、`git diff --check` 通过；逐包
  `cargo clippy --all-targets` 六个 crate 全部 **0 warning / 0 error**；`cargo clippy --workspace
  --all-targets` **0 warning / 0 error**；`cargo test --workspace` **269 通过 / 0 失败**
  （agent 44、app 67 + 集成 2、core 38、prompt-lab 2、store 96、xtask 20）；`pnpm lint` **0 问题**、
  `pnpm test` **10 通过 / 0 失败**。
- 本批改动的文件：`crates/xtask/src/main.rs`、`docs/08-giant-file-split-plan.md`。已搬迁的代码一行
  未动（工作区相对 HEAD 仍是十批留下的 16 个 app 文件 + 工具 + 本文档）；另一线程的
  `crates/nekowriter-prompt-lab` 与 `/tmp/nw-story-run` 未碰，无密钥进入响应或日志。

---

### 2026-09-16 P2.1：`store/lib.rs` 的命令层 → `store/command.rs`（只移动，未提交）

Phase 2 的第一批，也是第一次在 `store` 里做「同一文件 + 新模块」的搬迁（`--scope crates`）。
搬的是命令层与五个 ID/事务 helper：`CommandReceipt`、`impl Store` 里的
`replay_after_receipt_conflict`、`payload_hash`、`replay_or_conflict`、`insert_receipt`、`db_id`、
`parse_id`、`parse_optional_id`、`transaction_error`、`Command`、`Notify`、`impl Notify`、`impl
Command`、`approval_mode_name`——**17 个定义项、项内 130 行**，共 250 行的新文件（含 6 行模块文档头）。

- 声明与规模：`--from crates/nekowriter-store/src/lib.rs`，`--into` 同路径 + 新文件
  `crates/nekowriter-store/src/command.rs`（`--allow` 默认就是 `--from ∪ --into`，本批没另加）。
  `lib.rs` **7,443 → 7,231 行**（`numstat +13 −225`），`command.rs` **0 → 250 行**。
- **单批隔离**（`--base-state /tmp/nw-split-state/P2.0`）：**MOVE-ONLY OK**，定义项
  **123 → 123**、项内行 **6,764 → 6,764**、项外自由行 **180 → 186**、残留 **0**、放行新增注释 **6 行**
  （就是新文件的模块文档头）。
- **整栈核对**（`--base HEAD`，18 个 `--into`：Phase 1 的 16 个 app 文件 + `store/lib.rs` +
  `store/command.rs`）：**MOVE-ONLY OK**，定义项 **315 → 315**、项内行 **10,265 → 10,265**、
  项外自由行 **309 → 444**、残留 **0**、放行新增注释 **135 行**。逐文件 `main.rs +11 −4,136`、
  `store/lib.rs +13 −225`、`command.rs +250`，其余 15 个 app 文件与 P1.10 记录相同。
- **定义项集合**：`--list-items` 两侧都是 123 项，**只留签名末段的定义名去重后也是 123 / 123，
  两个方向的差集都是 0**——即 `command.rs` 里的 17 项全部来自 `lib.rs`，搬迁没有新增或删除定义。
- **独立复刻（不依赖 xtask）**：（一）归一化多重集（丢 `use`/`mod;`/`cfg(test)`/容器头与括号，
  签名合并成一行并去掉空白与折行尾逗号）——**搬迁前独有 0 行**（一行都没丢），搬迁后独有 6 行，
  逐条都是 `command.rs` 的 `//!` 模块文档头；（二）原始多重集（只 trim、只丢空行）逐类计数——
  搬迁前独有 9 行 = **接线 9 + 签名折行 13 + 可见性前缀 7 + 代码 0**，搬迁后独有 25 行 =
  **接线 25 + 注释 6 + 签名折行 26 + 可见性前缀 7 + 代码 0**。两个方向的代码行都是 **0**。
  （上一轮那个「项分解收支」脚本对 `command.rs` 差 0；对 `lib.rs` 两侧同为 **−31**，缺口是脚本
  没把泛型容器头 `impl<'a> X<'a> {` 与容器闭合括号当项外自由行——**两侧一致，与搬迁无关**；
  归一化多重集那条已经给出「一行未丢」的直接证据，不再依赖这个脚本。）
- 判据这次**没有暴露新缺口**，工具一行未改。本批唯一需要现场决定的事：`lib.rs` 的
  `find_receipt` 仍要构造 `CommandReceipt`，所以它的 4 个字段从私有改成 `pub(crate)`
  （`CommandReceipt` 结构体本身也是 `pub(crate)`）；`replay_after_receipt_conflict` 只有
  `Command::finish` 一个调用点（同在 `command.rs`），因此**保持私有**；`finish` 被
  `preparation.rs` / `prompts.rs` 调用，改成 `pub(crate)`。`lib.rs` 的再导出是两条：10 个名字的
  `pub(crate) use command::{...}`（兄弟模块的 `use super::{...}` 一行不改）+ 一行私有
  `use command::CommandReceipt;`。导入表清扫：  `lib.rs` 删掉 `pin::Pin`、`sha2::{Digest, Sha256}`、`DatabaseTransaction`、`TransactionError`
  与 `serde::de::DeserializeOwned`（这五个名字现在只被 `command.rs` 用）；
  `ApprovalMode` 只被测试模块用，因此从文件顶端下沉进 `mod tests` 的导入表。
- 调用方未动：相对 HEAD（含未跟踪）共 **20 个路径**（Phase 1 的 16 个 app 文件 + 本批的
  `store/lib.rs` 与 `command.rs` + 工具 + 本文档），**声明外 0**——`store` 其余 21 个顶层源文件
  与 28 个 `entity/*.rs`、app 的 crate 根另外 10 个域模块（`dashboard.rs`/`knowledge.rs`/
  `lifecycle.rs`/`manuscript.rs`/`model_settings.rs`/`operations.rs`/`prompts.rs`/`recycle.rs`/
  `scheduler.rs`/`templates.rs`）自 HEAD 以来一字未改；`nekowriter-prompt-lab` 与 `/tmp/nw-story-run`
  未碰，无密钥进入响应或日志。
- 门禁（2026-09-16，本批）：`cargo fmt --all -- --check` 通过、`git diff --check` 通过；逐包
  `cargo clippy --all-targets` 六个 crate 全部 **0 warning / 0 error**；`cargo clippy --workspace
  --all-targets`（强制全量重编译）**0 warning / 0 error**；`cargo test --workspace` **269 通过 /
  0 失败**（agent 44、app 67 + 集成 2、core 38、prompt-lab 2、**store 96**、xtask 20）；`pnpm lint`
  **0 问题**、`pnpm test` **10 通过 / 0 失败**。
- **下一批基准快照：`/tmp/nw-split-state/P2.1`（95 个 Rust 源文件）**；P2.2 的单批隔离用
  `--base-state /tmp/nw-split-state/P2.1`，仍然 `--scope crates`，`--from` / `--into` 都要把
  `crates/nekowriter-store/src/lib.rs` 列进去（它还会是 P2.2 的被拆文件）。
  **实测的单批命令**（**不要给 `--allow`**：留空时检查器把 `--from ∪ --into` 当默认允许集合，正好
  就是本批声明的两个路径。一旦手动给任何一个 `--allow`，这个默认集合会被整体替换，`lib.rs` /
  `projects.rs` 反而被算成「声明外改动」——整栈核对那一条则必须显式把 19 个搬迁文件与工具都列进
  `--allow`）：

  ```
  cargo run -q -p xtask -- check-move-only --scope crates \
    --base-state /tmp/nw-split-state/P2.1 \
    --from crates/nekowriter-store/src/lib.rs \
    --into crates/nekowriter-store/src/lib.rs \
    --into crates/nekowriter-store/src/projects.rs
  ```

  三种佐证脚本（`--list-items` 双层差集、不依赖 xtask 的归一化复刻、原始多重集逐类计数）沿用
  P2.1 的做法，参数换成 `P2.1 → 工作区`、`--from lib.rs`、`--into lib.rs projects.rs` 即可。

### 2026-09-16 P2.2：项目与稿本层 → `store/projects.rs`（只移动，未提交）

按 §5 里 P2.2 界定的六段（`lib.rs` 当时的 326–390、390–937、901–931、932–978、1,115–1,202、
1,227–1,288 行；P2.1 记录里那张「剩下的区域」表已由本篇下面的现行表取代）把生产区里 P2.2 的几段
一次搬完：`impl Store` 里 14 个与项目/稿本相关的方法（`project_id_for_version` 到 `find_receipt`）、
`version_snapshot`、三个 payload 结构 + 两个 `From` impl、四个 `insert_*`、四个 `*_from_model`
——**28 个定义项、项内 661 行**，
落地成 827 行的新文件（含 7 行模块文档头）。这一批搬完后 `lib.rs` 生产区只剩 25 项 / 282 行，已经
是对着计划的收尾形状：**Phase 2 的生产代码拆分到此结束**，剩下的只有 P2.3（测试模块）与 P2.4
（形状测试路径表）。

- 声明与规模：`--from crates/nekowriter-store/src/lib.rs`，`--into` 同路径 + 新文件
  `crates/nekowriter-store/src/projects.rs`。`lib.rs` **7,231 → 6,440 行**（单批 `numstat +18 −809`），
  `projects.rs` **0 → 827 行**。
- **单批隔离**（`--base-state /tmp/nw-split-state/P2.1`）：**MOVE-ONLY OK**，定义项
  **106 → 106**、项内行 **6,634 → 6,634**、项外自由行 **151 → 158**、残留 **0**、放行新增注释 **7 行**
  （就是新文件的模块文档头）。
- **整栈核对**（`--base HEAD`，19 个 `--into`：Phase 1 的 16 个 app 文件 + `store/lib.rs` +
  `store/command.rs` + `store/projects.rs`）：**MOVE-ONLY OK**，定义项 **315 → 315**、项内行
  **10,265 → 10,265**、项外自由行 **309 → 451**、残留 **0**、放行新增注释 **142 行**。逐文件
  `store/lib.rs +25 −1028`（含 P2.1 的 −225）、`command.rs +250`、`projects.rs +827`。
- **定义名集合的双向差集**：`--list-items` 两侧去重后都是 **106 / 106**，两个方向差集都是 **0**；
  即 `projects.rs` 里那 28 项全部来自 `lib.rs`，没有新增或删除定义。
- **独立复刻（不依赖 xtask）**：（一）归一化多重集——**搬迁前独有 0 行**（一行未丢），搬迁后独有
  7 行，逐条都是 `projects.rs` 的 `//!` 模块文档头；（二）原始多重集（只 trim、只丢空行）逐类计数——
  搬迁前独有 17 行 = **接线 11 + 签名折行 6 + 代码 0**，搬迁后独有 50 行 = **接线 32 + 注释 7 +
  签名折行 11 + 代码 0**。两个方向的代码行都是 **0**。判据这次**没有暴露新缺口，工具一行未改**
  （本批第一次跑就 MOVE-ONLY OK，与 P1.9、P2.1 一样）。
- 可见性与接线（本批唯一的现场决定）：`find_receipt` 被 `command.rs` 调，改成 `pub(crate)`；
  被兄弟模块直接调用的五个自由函数（`insert_document_slots`、`insert_novel_version`、
  `insert_workflow`、`project_from_model`、`workflow_from_model`）改成 `pub(crate)` 并由 `lib.rs` 的
  `pub(crate) use projects::{…}` 接回，兄弟模块的 `use super::{…}` / `use crate::{…}` 一行未改；
  `insert_project_bundle`、`version_snapshot`、三个 payload 结构、`version_from_model`、
  `project_bundle_from_models` 只在 `projects.rs` 内用，**保持私有**（不多给可见性）。
- 导入表作业（都是接线行）：`lib.rs` 顶端的 `nekowriter_core` 导入从 18 个名字收到 **3 个**
  （`ProjectId`、`VersionId`、`utc_now_millis`），`sea_orm::{…}` 从 12 个收到 **4 个**（另加
  `prelude::TextUuid`），`serde` 去掉 `de::DeserializeOwned`，`sha2` 与 `uuid::Uuid` 从文件顶端
  下沉进 `mod tests`；测试模块补上它真正用的
  `ChapterId`/`DocumentId`/`ProjectProgress`/`ReviewId`/`RunId`、`IntoActiveModel`/`QueryFilter`/`Set`
  与 `entity::{chapter, document, document_revision, novel_version, workflow_instance}`。
  另外在 `normalize_api_key` 与 `#[cfg(test)]` 之间补了一个空行（纯排版，与仓库其余模块一致）。
- 调用方未动：相对 HEAD（含未跟踪）共 **21 个路径**（Phase 1 的 16 个 app 文件 + `store/lib.rs` +
  `command.rs` + `projects.rs` + 工具 + 本文档），**声明外 0**——`store` 其余 21 个顶层源文件与 28 个
  `entity/*.rs`、app 的 crate 根 10 个域模块自 HEAD 以来一字未改；`nekowriter-prompt-lab` 与
  `/tmp/nw-story-run` 未碰，无密钥进入响应或日志。
- 门禁（2026-09-16，本批，实际跑过）：`cargo fmt --all -- --check` 通过、`git diff --check` 通过；逐包
  `cargo clippy --all-targets` 六个 crate 全部 **0 warning / 0 error**；`cargo clippy --workspace
  --all-targets` 前先 `touch` 了五个 crate 的源码强制重编译（输出里六个 crate 都是 `Checking`）
  **0 warning / 0 error**；`cargo test --workspace` **269 通过 / 0 失败**（agent 44、app 67 +
  集成 2、core 38、prompt-lab 2、**store 96**、xtask 20），store 单独再跑一次也是 **96 通过 / 0 失败**
  （`-- --list` 确认无 doc-test）；`pnpm lint` **0 问题**、`pnpm test` **10 通过 / 0 失败**（2 个文件）。
- **下一批基准快照：`/tmp/nw-split-state/P2.2`（96 个 Rust 源文件）**，与工作区逐字节一致
  （`diff -rq` 只在快照按设计不收录的 `Cargo.toml` / `prompts/` 上报差异）。P2.3 的单批隔离用
  `--base-state /tmp/nw-split-state/P2.2`，`--scope crates`，`--from` / `--into` 里要把
  `crates/nekowriter-store/src/lib.rs` 与 P2.3 新建的 `src/tests/` 文件全部列出——与 P1.10 一样，
  被拆的测试模块会变成多个 `mod tests` 包装，这是判据在 P1.10 已经处理过的形状。

### P2.2 之后 `store/lib.rs` 剩下的区域（现行 6,440 行）

生产区现在是 **1–429 行**（25 个定义项 / 项内 282 行 / 项外自由行 42 行 / 接线 105 行），
测试区是 **431–6,440 行**（`#[cfg(test)]` 在 431，53 项 / 项内 5,691 行 / 项外自由行 76 行 / 接线
243 行，与 P2.1 时一字未动），中间第 430 行是空行。（接线行按快照表的同一口径取：总行 − 项内 −
项外，因此含空行。）

| 行区间 | 内容 | 去向 |
| --- | --- | --- |
| 1–9 | 模块文档（1–4）与 `use std::{path::{Path, PathBuf}, sync::Arc}`（6–9） | 留（crate 根的接线） |
| 11–17 | 其余导入表（`nekowriter_core` 3 个名字、`sea_orm::{…}` 4 个 + `prelude::TextUuid`、`serde`、`thiserror`、`tokio::sync::broadcast`、`url::Url`） | 留 |
| 19–31 | `SCHEMA_VERSION` 与 9 个命令/角色常量（含 `SCHEMA_VERSION` 的两行文档注释） | 留（搬走的函数仍靠 `super::` 取这些常量，保持私有即可） |
| 33–56 | 22 条 `mod` 声明 | 留 |
| 58–98 | 13 条 `pub use` / `pub(crate) use` 再导出 + 尾部的 `use command::CommandReceipt;` | 留 |
| 100–107 | `Store`（含 `#[derive(Clone)]`） | 留（对外类型） |
| 108–249 | `StoreError`（141 行，含 `#[derive(Debug, Error)]`） | 留（对外类型） |
| 250–313 | `impl Store`：`open`（22 行）、`close`、`schema_version`、`data_dir`、`lance` | 留（打开/关闭与访问器） |
| 314–354 | `CredentialPatch` + `impl CredentialPatch`（`merge` / `normalize` / `intent`） | 留（对外可见类型，`settings.rs` 用） |
| 356–364 | `CredentialIntent` | 留 |
| 365–376 | `impl<'de> Deserialize<'de> for CredentialPatch` | 留 |
| 378–415 | `validate_api_base_url` | 留（`knowledge.rs` / `settings.rs` 用） |
| 416–429 | `normalize_api_key` | 留（同上） |
| 431–6,440 | `#[cfg(test)] mod tests`：53 项 / 项内 5,691 行（含共享 harness） | **P2.3** 拆进 `src/tests/`（分三次提交：a 项目/设置/schema、b 准备/正文、c 审核/事件/回收/LanceDB/形状） |

生产区已经没有「按域可搬的段」了。P2.3 的入口现状：`lib.rs` 的测试模块从 431 行到文件尾共
6,010 行、**53 个定义项**——按项逐条数出来是 **46 个测试函数**（44 个 `#[tokio::test]` + 2 个
`#[test]`：`api_base_url_rule_is_one_rule_for_both_settings`、
`transaction_external_recheck_has_a_single_owner`）加 **7 个共享 harness**（`functions()`、
`in_flight_preparation_run`、`delegated_background_review`、`drafting_chapter_body_run`、
`dispatched_body_tool_call`、`persisted_protocol_error_count`、`published_protocol_error_count`），
与 `--list-items` 的 53 项逐条吻合（46 + 7 = 53，最大的单一项是 924 行的
`approved_preparation_artifacts_fold_to_complete_plaintext_after_the_last_review`）。
P2.3 执行前的预估是「`lib.rs` 431 行、`src/tests/` 6,010 行、8 个文件」（431 是按 429 + 空行 +
`#[cfg(test)] mod tests;` 估的，实际是 432：生产区 429 + 空行 + 两行模块声明）；**实测结果与
偏差见下面 P2.3 的记录**。

### 2026-09-16 P2.3（含 P2.4）：测试模块 → `store/src/tests/`（Phase 2 收尾，未提交）

Phase 2 的最后一批，也是整个拆分里唯一一批「搬测试」。搬完后 `lib.rs` **6,440 → 432 行**
（`numstat +1 −6,009`；对 HEAD 是 `+15 −7,026`），只剩生产区 429 行 + 空行 +
`#[cfg(test)] mod tests;`；测试落到 **`src/tests/` 的 15 个文件 / 6,081 行**。

**先说结论：这一批不是「只移动」，判据报 81 行残留，而且这不是判据的缺口。**

- 原因：53 个项原来在 `mod tests` 里（缩进 4），搬到 `src/tests/*.rs` 后文件顶层缩进是 0
  （rustfmt 对 out-of-line 模块文件一律按列 0 格式化；同仓库的 `http/handlers/system.rs` 是同一
  现象），行宽预算变大，rustfmt 把原本折行的调用链合并（例：`.submit_chapter_final_review(
  extra.id, Uuid::new_v4(), extra_body.workflow.state_revision)` 从 3 行并成 1 行）。判据比的是
  「归一化后的有序行」，合并/拆开必然报差异——它报得对。这条口径写进了 §3。
- 真正的内容改动只有一处：**P2.4** 的 `include_str!` 路径修正 9 行（`"lib.rs"` →
  `"../lib.rs"`，同理 8 个兄弟模块）；其中 `prompt_templates.rs` 与 `project_settings.rs` 两条
  加 `../` 后超过行宽，被 rustfmt 折成三行。

**文件划分（实测 15 个文件，不是计划里估的 8 个）**：

| 文件 | 项数 | 项内行 | 文件行 |
| --- | ---: | ---: | ---: |
| `mod.rs`（导入表 + 7 个共享 harness） | 7 | 295 | 373 |
| `agent_messages.rs` | 4 | 409 | 440 |
| `chapters.rs` | 3 | 510 | 534 |
| `commands.rs` | 2 | 121 | 143 |
| `events.rs` | 4 | 365 | 395 |
| `fold.rs` | 1 | 924 | 942 |
| `lance.rs` | 1 | 31 | 37 |
| `model_requests.rs` | 3 | 292 | 307 |
| `preparation.rs` | 3 | 345 | 364 |
| `projects.rs` | 4 | 293 | 315 |
| `review.rs` | 4 | 858 | 885 |
| `schema.rs` | 5 | 292 | 312 |
| `settings.rs` | 3 | 179 | 195 |
| `shape.rs` | 1 | 49 | 70 |
| `writing.rs` | 8 | 728 | 769 |
| 合计 | **53** | **5,691** | **6,081** |

文件名对齐 `store` 自己的模块名，两个例外都在文档里写明了原因：`fold.rs` 是那一条 924 行的
端到端用例（它单独成文件，才能让每个文件都 ≤ 1,300 行，实测最大 942 行）；`lance.rs` 不能叫
`lancedb.rs`——子模块名会遮蔽同名的外部 crate，而用例体里就有 `lancedb::connect(...)`（这一条
是编译报错现场发现的，`tests/mod.rs` 的 `mod` 声明旁留了一行注释）。

**三层证据（快照 + 整栈 + 独立复刻）**

- 单批隔离（`--base-state /tmp/nw-split-state/P2.2`、`--scope crates`、`--from
  crates/nekowriter-store/src/lib.rs`、`--into` 同一个 `lib.rs` + 15 个 `crates/nekowriter-store/src/tests/*.rs`，
  **不给 `--allow`** 以保留默认的 `--from ∪ --into` 允许集合）：退出码 **1**，
  定义项 **78 → 78**、项内行 **5,973 → 5,960**、项外自由行 **118 → 168**、残留 **81 行**
  （另有允许的新增注释 **50 行**——15 个文件的文档头）。
- 整栈核对（`--base HEAD`，34 个 `--into`）：退出码 **1**，定义项 **315 → 315**、项内行
  **10,265 → 10,252**、项外自由行 **309 → 501**、残留 **81 行**（与单批完全相同，即
  **前九批加到「审」上仍是 0 残留**）、放行新增注释 **192 行**；**声明外 0**。
- 独立复刻（不依赖 xtask）：自己按 §1 的口径切项 + 复刻 `normalize_block`，逐项做归一化行多重集差
  ——**− 47 行 / + 34 行、`~` 行序标记 0 项，合计 81 行，与工具逐数字吻合**。归类：
  **`include_str!` 路径行 − 9 / + 9（P2.4 声明的位置修正）** + 其余 **− 38 / + 25（rustfmt 重排）**，
  集中在 10 个项：`transaction_external_recheck…` 24 行、`approved_preparation_artifacts_fold…`
  15 行、`outline_submission…` 12 行、`persisted_events…` 6 行、`chapter_final_rejection…` 6 行，
  其余 5 项各项 3 行。
- **逐项 token 级比对（关键证据，不依赖行切分）**：把每一项在搬迁前后各自压成「去空白 + 去掉紧邻
  `)`/`]`/`}` 的尾逗号」的字符串后逐字节比较——**52 / 53 项完全相同**；唯一不同的一项是 `shape`，
  差异就是那 9 条路径字符串（压后长度 1,235 → 1,262，**+27 = 9 × 3**）。也就是说 81 行残留里
  **0 行是行为或内容改动**。
- 定义项名字集合：两侧 `--list-items` 都是 **78 项 / 去重后 78 个名字**（25 个生产项 + 53 个测试项），
  两个方向的差集都是 **0**；测试区单独看也是 53 / 53、差集 0。
- 测试身份：`cargo test -p nekowriter-store -- --list` 前后都是 **96 个用例**，**叶子函数名集合两个方向
  差集都是 0**（模块前缀从 `tests::name` 变成 `tests::<域>::name`，正是 §3 第四道允许的形状），
  没有任何用例新增、删除或改名，断言一行未动。
- 调用方未动：相对 HEAD（含未跟踪）共 **36 个路径** = Phase 1 的 16 个 app 文件 + `store` 的 3 个
  生产文件 + 15 个 `tests/*.rs` + `xtask` + 本文档，**声明外 0**；`store` 其余 21 个顶层源文件、
  28 个 `entity/*.rs`、app 的 crate 根 10 个域模块自 HEAD 以来一字未改；`nekowriter-prompt-lab`
  与 `/tmp/nw-story-run` 未碰，无密钥进入响应或日志。`xtask` 这一批**一行未改**（判据报的 81 行是
  真实文本差异，不是缺口）。

**门禁（2026-09-16，本批实测）**：`cargo fmt --all -- --check` 通过（新文件与 `lib.rs` 都是 fmt 后
的状态）、`git diff --check` 通过；逐包 `cargo clippy --all-targets` 六个 crate 全部 **0 warning /
0 error**（第一轮曾报 2 条 `clippy::doc_markdown`——`lance.rs` 的模块头里 `LanceDB` 没加反引号，已修）；
`cargo clippy --workspace --all-targets` 先 `touch` 五个 crate 源码强制重编译（输出里六个 crate 都是
`Checking`）**0 warning / 0 error**；`cargo test --workspace` **269 通过 / 0 失败**（agent 44、app 67
+ 集成 2、core 38、prompt-lab 2、**store 96**、xtask 20），store 单跑也是 **96 通过 / 0 失败**；
`pnpm lint` **0 问题**、`pnpm test` **10 通过 / 0 失败**（2 个文件）。

**本批文件清单**：`crates/nekowriter-store/src/lib.rs`（6,440 → 432 行）、新建
`crates/nekowriter-store/src/tests/{mod,agent_messages,chapters,commands,events,fold,lance,model_requests,preparation,projects,review,schema,settings,shape,writing}.rs`、
`docs/08-giant-file-split-plan.md`。

### Phase 2 收尾结论与 Phase 3 入口

- **Phase 2 结束**：`lib.rs` **7,443 → 432 行**（生产区 429 + 接线），`store/src/` 现在 39 个 `.rs`
  文件，最大的生产文件是 `preparation.rs`（13,205 行，Phase 3 的对象）。四批一共搬出 45 个生产项
  （P2.1 17 项 / P2.2 28 项）与 53 个测试项。
- **下一批基准快照：`/tmp/nw-split-state/P2.3`（111 个 Rust 源文件）**，与工作区逐字节一致；
  Phase 3 的单批隔离用 `--base-state /tmp/nw-split-state/P2.3`，`--scope crates`。
- **Phase 3 的 V0 基线数字（就是本批收工那一轮门禁，已记在上面）**：fmt 通过、workspace clippy
  **0/0**、`cargo test --workspace` **269 通过 / 0 失败**、`cargo test -p nekowriter-store -- --list`
  **96 个用例**。Phase 3 每批回到这里对比。
- 入口条件（§5 里那份）当时剩两条待决：批次粒度，以及「要不要在动它之前先补一层端到端安全网」。
  前者已在下一节**定案**（切片单位 = 一个目标模块 = 一层，预计 18 批，替代原先的「一次 1–2 个
  定义项」——290 项按那个粒度是 150 批以上，成本不成比例）；后者仍是**计划外的新工作**，
  不动它就靠项级判据 + 每批跑两个 crate 的测试兜底。

### 2026-09-16 Phase 3 批次粒度（定案，替代原先的「一次 1–2 个定义项」）

`preparation.rs` 的项清单基线是 **290 项 / 项内 11,634 行**。若按原先写的「一次 1–2 个定义项」
执行就是 **150 批以上**，而每批都要跑两层判据 + 两个 crate 的测试 + 一份执行记录，成本与收益
不成比例。重算后的结论：

- **切片单位 = 一个目标模块（一层 / 一个主题）**，即 §5 表里 P3.3–P3.17 的每一行；P3.12（~2,200 行）
  拆成 2 次提交。加上 P3.1 与 P3.2，**预计 18 批**，单批项内行 38–1,200 行，目标文件 ≤ 1,300 行。
- **为什么这个尺寸仍然「可独立验证、失败能定位到项」**：判据的单位是**定义项**而不是文件，
  残留永远带项名（`- [async fn foo(…)] <行>`、`~ [async fn foo(…)] 行序改变`），`--list-items`
  还给出**定义名双向差集**，编译与测试失败也带项名与行号。也就是说批量**只影响回滚粒度，
  不影响定位精度**：返工成本的上界是「重放那一批的搬迁」。
- **定位变难时怎么缩（预先定好，不在现场再想）**：① 先用 `--list-items` 的定义名差集把失败圈到项名；
  ② 判据报的项名互相掩盖（例如缩进层变化引来一片 rustfmt 重排）时，用 §3 那条「跨缩进层例外」
  的**逐项 token 级比对**把排版与语义分开；③ 仍定位不了就**就地缩到「该批里最小的可独立编译子集」**：
  先按项名二分，再缩到「单项 + 它的私有依赖闭包」，子集继续用同一个基准快照，不重跑整栈核对；
  ④ 只有某一批连缩两次都定不下来，才把该批的移动回滚重做，并把这件事记进本节。
- **不缩的代价也说清**：批次越大，「一批里混进真行为差异」的窗口越大，而兜底只有
  `cargo test -p nekowriter-store -p nekowriter-app`（`preparation.rs` 自身 0 测试）。所以
  §5 入口条件第 5 条（要不要先补端到端安全网）依旧是本阶段唯一悬着的人决策。

### 2026-09-16 P3.1：事件写入正典 → `store/event_log.rs`（Phase 3 第一批，未提交）

**目标模块的偏离（先说清再动手）**：计划原定新建 `store/events.rs`，执行时改成**并进已存在的
`store/event_log.rs`**（158 行：读侧 + 保留期 + 唤醒通知）。三条理由：① `events` 表已经有
一个所有者，读、写、清理、通知同域才是一个事实来源一个模块；② `store/src/` 里已有
`event_log.rs` 与 `event_types.rs`，再加一个名字近似的 `events.rs` 会让 `rg event` 命中三个模块，
长期是负担；③ 3 个函数 63 行单独成文件，对 `preparation.rs` 的瘦身没有额外收益（Phase 3 的目标是
缩小 `preparation.rs`，不是增加文件数）。`event_log.rs` 的模块文档**只追加**了两行（写入侧归属），
既有注释一行未改写。

**第二处偏离**：计划写的是「`lib.rs` 里 `pub(crate) use events::{…}`」。实际调用方全部按
`preparation::insert_app_event` / `super::preparation::{insert_event, …}` 引用（`projects.rs`、
`knowledge.rs`、`settings.rs`、`prompts.rs`、`prompt_templates.rs`、`project_settings.rs`、
`system_operations.rs`、`model_requests.rs`、`protocol_correction.rs`、`recycle.rs`、`version_copy.rs`
共 11 个兄弟模块），且 `lib.rs` 的 `pub use preparation::{…}` 不含这两个名字。所以再导出落在
`preparation.rs`：`pub(crate) use crate::event_log::{insert_app_event, insert_event};`——11 个调用方、
`preparation.rs` 内部 121 处调用、app 侧全部一行未改。`lib.rs` 本批**一行未动**。

**搬迁量**：3 个定义项（`insert_event` 12 行、`insert_app_event` 12 行、`insert_event_row` 14 行，
共 **38 个项内行**）/ 原文件区间 13142–13205 共 **64 行**（63 行代码 + 1 空行），按字节原样切过去
（搬运脚本对每个边界做硬断言：前一行空行、三条签名、`insert_event_row` 的私有性、末行 `}`）。
`insert_event_row` 保持私有；`preparation.rs` 顺手删掉一条因搬迁而失效的导入
（`super::entity::event` 在该文件里只被 `insert_event_row` 用）。

| 文件 | 行数 | 对 HEAD 的 numstat |
| --- | ---: | --- |
| `crates/nekowriter-store/src/preparation.rs` | 13,205 → **13,144** | +6 −67 |
| `crates/nekowriter-store/src/event_log.rs` | 158 → **229** | +73 −2 |

**三层证据**

- 单批隔离（`--base-state /tmp/nw-split-state/P2.3`、`--scope crates`、`--from`/`--into` 都是
  `preparation.rs` + `event_log.rs`，**不给 `--allow`**）：退出码 **0，MOVE-ONLY OK**；
  定义项 **301 → 301**、项内行 **11,737 → 11,737**、项外自由行 **390 → 395**、残留 **0**、
  放行新增注释 **5 行**（3 行模块文档 + 2 行再导出说明）；声明内改动只有那两个文件。
- 整栈核对（`--base HEAD`，4 个 `--from` / 36 个 `--into`）：退出码 **1**，定义项 **616 → 616**、
  项内行 **22,002 → 21,989**、项外自由行 **699 → 896**、残留 **81 行**、放行新增注释 **197 行**。
  这 81 行**就是 P2.3 已逐项归因的那批 rustfmt 重排**（`approved_preparation_artifacts_fold…` 等
  10 项）；P3.1 加入后**一行未增，即本批自己的残留是 0**。
- 独立复刻（不依赖 xtask，`/tmp/nw-rule/replicate-p31.py`）：归一化多重集**搬迁前独有 0 行**
  （一行未丢），搬迁后独有 **5 行**全是注释（2 行 `//` + 3 行 `//!`）；原始多重集逐类计数——
  搬迁前独有 4 行全部是**接线**，搬迁后独有 8 行接线 + 5 行注释，**两个方向的代码行都是 0**。
- 定义名双向差集（`--list-items`）：两侧都是 **301 项**，两个方向差集 **0**；搬过去的三个名字
  （带完整签名）都在搬迁后侧。**可见性不参与项 key**（前缀被归一化掉），所以「`insert_event_row`
  保持私有」不靠判据而靠两件事：搬运脚本断言过它在两侧的签名逐字为 `async fn insert_event_row(`，
  且搬迁后 `cargo clippy --all-targets` 与 `cargo test` 均无 `unused` / `dead_code` 告警。
- 测试身份：本批没有搬测试，机械依据是
  `diff -rq crates/nekowriter-store/src/tests /tmp/nw-split-state/P2.3/crates/nekowriter-store/src/tests`
  **无输出（逐文件一致）**，且只有两个非测试文件被改；`cargo test -p nekowriter-store -- --list`
  仍是 **96 个用例**。
- **源码形状测试无需跟改（与 §4 地雷 1 对照）**：`event_types.rs` 的
  `every_written_event_type_is_in_the_manifest` 是**递归扫描 `src/` 全目录**取写库点（不是硬编码
  文件表），并且用「`insert_event(` 前紧邻 `fn` 则为定义处」的条件跳掉定义行——所以搬到
  `event_log.rs`、以及后续 `preparation/*.rs` 的写库点它都会自动覆盖，本批不改测试也仍然有效。
- 调用方未动：`git status --porcelain -uall` 共 **38 个路径** = P1 的 16 个 app 文件 + P2 的 18 个
  store 文件（`lib.rs`、`command.rs`、`projects.rs` + 15 个 `tests/*.rs`）+ 判据轮改的 `xtask` +
  每批都追加的 `docs/08` + 本批的 `event_log.rs` 与 `preparation.rs`，**声明外 0**。11 个引用
  `preparation::insert_*` 的兄弟模块与 app 侧一行未改；`nekowriter-prompt-lab` 与 `/tmp/nw-story-run`
  未碰，无密钥进入响应或日志。
- 工具：**一行未改**（`crates/xtask/src/main.rs` 仍是 2,133 行）。本批残差形状（同缩进层之间搬、
  不触发 rustfmt 重排）与 §3 预期一致。

**Phase 3 的 V0 数字**（= P2.3 收工那一轮，也是本批的对照基线）：`cargo fmt --all -- --check` 通过、
`cargo clippy --workspace --all-targets` **0 warning / 0 error**、`cargo test --workspace`
**269 通过 / 0 失败**、`cargo test -p nekowriter-store -- --list` **96 个用例**。

**门禁（2026-09-16，本批实测）**：`cargo fmt --all -- --check` 通过、`git diff --check` 通过；
逐包 `cargo clippy --all-targets` 六个 crate 全部 **0 warning / 0 error**（`nekowriter-store` 第一轮
报过 1 条 `unused import: event`——搬迁后失效的导入，已删，之后 0/0）；`cargo clippy --workspace
--all-targets` 先 `touch` 六个 crate 源码强制重编译（输出里六个 crate 都是 `Checking`）**0/0**；
`cargo test --workspace` **269 通过 / 0 失败**（agent 44、app 67 + 集成 2、core 38、prompt-lab 2、
**store 96**、xtask 20）；`pnpm lint` **0 问题**、`pnpm test` **10 通过 / 0 失败**（2 个文件）。

**本批文件清单**：`crates/nekowriter-store/src/preparation.rs`、`crates/nekowriter-store/src/event_log.rs`、
`docs/08-giant-file-split-plan.md`。**下一批基准快照：`/tmp/nw-split-state/P3.1`（111 个 Rust 源文件）**，
与工作区逐字节一致（两个被改文件都 `diff -q` 通过）。**P3.2**：`preparation.rs` → `preparation/mod.rs`
纯改名（不进内容比对，只验路径与 `include_str!` 形状），本批留在 `preparation.rs` 里的再导出
随改名进入 `mod.rs`；此后按「一个目标模块一批」推进 P3.3–P3.17。

### 2026-09-16 P3.2：`preparation.rs` → `preparation/` 目录模块（未提交）

本批只做两件事：`git mv crates/nekowriter-store/src/preparation.rs
crates/nekowriter-store/src/preparation/mod.rs`（**13,144 行逐字节原样**），以及 `tests/shape.rs`
里那条 `include_str!` 的路径与显示名跟着改成 `../preparation/mod.rs`（外加 2 行说明注释）。
**没有搬任何代码、没有新建层文件**——层文件从 P3.3 开始按「一个目标模块一批」落地。

**模块路径为什么稳定（回答「要不要 `pub(crate) use`」）**：Rust 对 `lib.rs` 里的
`mod preparation;` 会自动解析 `preparation.rs` 或 `preparation/mod.rs`（二者只能有一个），
所以模块名与所有项目路径一个字符都不用变：11 个兄弟模块的 `super::preparation::{…}` /
`preparation::insert_app_event`、app 侧对 `nekowriter_store::preparation::*` 的用法全部照旧，
**不需要任何再导出或门面**（P3.1 那条 `pub(crate) use crate::event_log::{…}` 也随改名留在
`mod.rs` 里）。后续 P3.3+ 把项目搬进层文件时才需要在 `mod.rs` 补
`pub(crate) use`，因为那才会改变「谁定义这个名字」。

| 路径 | 变化 |
| --- | --- |
| `store/preparation.rs` | 删除（13,144 行） |
| `store/preparation/mod.rs` | 新建，13,144 行，与旧文件逐字节相同 |
| `store/tests/shape.rs` | 70 → 72 行（+2 行注释、1 行路径行改写） |

**三层证据**

- 单批隔离（`--base-state /tmp/nw-split-state/P3.1`、`--scope crates`、`--from preparation.rs +
  tests/shape.rs`、`--into preparation/mod.rs + tests/shape.rs`，**不给 `--allow`**）：退出码 **1**，
  定义项 **288 → 288**、项内行 **11,651 → 11,651**、项外自由行 **372 → 374**、**残留 2 行**、
  放行新增注释 **2 行**。那 2 行残差**就是声明的路径修正**，工具点名到位：
  `- [fn transaction_external_recheck_has_a_single_owner()] ("preparation.rs", include_str!("../preparation.rs")),`
  / `+ […] ("preparation/mod.rs", include_str!("../preparation/mod.rs")),`——与 P2.4 同类，
  属「声明的位置修正」而不属行为变更（§3 第三道允许）。
- **窄声明复核（把改名与路径修正分开看）**：只声明 `--from preparation.rs --into preparation/mod.rs`
  时，定义项 **287 → 287**、项内行 **11,596 → 11,596**、项外自由行 **361 → 361**、**残留 0**
  —— 即改名本身是一次零残差搬迁；那 2 行残差全部来自 shape.rs 的路径修正。
- 整栈核对（`--base HEAD`，4 个 `--from` / 36 个 `--into`）：退出码 **1**，定义项 **616 → 616**、
  项内行 **22,002 → 21,989**、项外自由行 **699 → 898**、**残留 81 行**、放行新增注释 **199 行**。
  81 行**与 P3.1 那一轮完全相同**：P2.3 已逐项归因的那批 rustfmt 重排里，包含本批改写的那一对
  `include_str!` 行（P2.4 当时算的就是一对），本批只是把它的内容换新，**对数不变，本批新增残留 0**。
- 独立复刻（不依赖 xtask，`/tmp/nw-rule/replicate-p32.py`）：① **按声明替换后**（把搬迁后侧的
  `preparation/mod.rs` 换回 `preparation.rs`）归一化多重集——搬迁前独有 **0 行**，搬迁后独有
  **2 行**全是新增注释；② **不替换**的原始多重集逐类计数——搬迁前独有 **1 行代码**
  （`("preparation.rs", include_str!("../preparation.rs")),`）、搬迁后独有 **1 行代码 + 2 行注释**
  （`("preparation/mod.rs", include_str!("../preparation/mod.rs")),`）——两侧的代码行都只有这一行，
  且它在替换后逐行相同（① 的 0 行就是证据）；③ 逐项 token 级比对（去空白 + 去紧邻 `)`/`]`/`}`
  的尾逗号）：同名项 **242 / 242、逐项不同 0 项**（242 是这份独立切分器的口径，比判据的 287 少，
  因为它会把多行签名与嵌套项合并成一个块）。
- 定义名双向差集（`--list-items`）：两侧都是 **288 项**（287 个 `mod.rs` 项目 + 1 个 shape 用例），
  两个方向差集 **0**。
- 测试身份：`cargo test -p nekowriter-store` **96 通过 / 0 失败**，其中
  `tests::shape::transaction_external_recheck_has_a_single_owner` 单独跑也 **ok**（它嵌入的
  `preparation/mod.rs` 与改名前的 `preparation.rs` 逐字节相同，`functions(source)` 的切分结果不变，
  断言与白名单一字未改）；`--list` 仍 **96 个用例**。
- 调用方未动：`git diff --name-only HEAD` 只有 **5 个已跟踪路径**（`crates/nekowriter-app/src/main.rs`、`store/event_log.rs`、
  `store/lib.rs`、`store/preparation.rs`（本批删除）、`xtask/main.rs`），加未跟踪清单共 **39 个路径**，
  本批只声明 3 个（`preparation.rs`、`preparation/mod.rs`、`tests/shape.rs`），**声明外 0**
  —— 11 个兄弟模块、app 侧、`lib.rs` 都一字未改（`lib.rs` 的 `mod preparation;` 仍指同一模块）。
- 工具：**一行未改**（`crates/xtask/src/main.rs` 仍 2,133 行）。

**门禁（2026-09-16，本批实测）**：`cargo fmt --all -- --check` 通过、`git diff --check` 通过；
逐包 `cargo clippy --all-targets` 六个 crate 全部 **0 warning / 0 error**；
`cargo clippy --workspace --all-targets` 强制重编译（六个 crate 全 `Checking`）**0/0**；
`cargo test --workspace` **269 通过 / 0 失败**（agent 44、app 67 + 集成 2、core 38、prompt-lab 2、
**store 96**、xtask 20）；`pnpm lint` **0 问题**、`pnpm test` **10 通过 / 0 失败**。

**本批文件清单**：`crates/nekowriter-store/src/preparation.rs`（删）、
`crates/nekowriter-store/src/preparation/mod.rs`（新建）、`crates/nekowriter-store/src/tests/shape.rs`、
`docs/08-giant-file-split-plan.md`。**下一批基准快照：`/tmp/nw-split-state/P3.2`（111 个 Rust 源文件）**，
与工作区逐字节一致（`mod.rs` 与 `shape.rs` 都 `diff -q` 通过）。快照的写法要记一笔：写状态只在
判定通过后执行，所以这一次的**窄声明**跑（只比改名那一对、`shape.rs` 用 `--allow`）
带 `--write-state`，与本批的**正式声明**跑（288/288、残留 2 行）共用同一份工作区。
**P3.3**：引用与意见层 → `preparation/citations.rs`（`mod.rs` 现行 640–1266 区间，约 630 行），
这是第一次真正从 `mod.rs` 搬代码，也是第一次需要 `mod.rs` 里的 `pub(crate) use` 把搬走的项接回。

### 2026-09-16 P3.3：引用与意见层 → `preparation/citations.rs`（未提交）

**边界怎么定的**：计划那一行写的是 640–1266，实际按**项边界**取 **633–1267**：
起始点往前挪到 `enum CandidateCitationScope` 的文档注释（639 前一行，即 633），因为它是引用校验的
「引用范围」类型（`validate_chapter_body_feedback_citations` 的形参与两个分支都用它），
属于本层；计划里 P3.4 的行区间写到 639，正好把它劈成两半。结束点在
`bind_pending_feedback_to_existing_tool_for_pause` 的闭合大括号（1267），它之后就是
P3.4 的 `FreeAgentToolCall`。区间内**全部是顶层自由函数/枚举**（无 `impl` 容器、无模块级常量），
所以可以整段搬而不用拆容器。

**搬走什么**：计划列的 21 项 + `CandidateCitationScope` = **22 个定义项**，区间 635 行
（565 行代码 + 49 行注释 + 21 行空行），按字节原样切走。

| 路径 | 行数 |
| --- | ---: |
| `store/preparation/mod.rs` | 13,144 → **12,523**（搬走 635 行，新增接线/注释 14 行，净 −621）|
| `store/preparation/citations.rs` | 新建 **667** 行（635 行搬迁内容 + 32 行模块文档与导入）|

**再导出（本批的核心问题）**：搬走后有 **17 项被块外引用**（mod.rs 自己或兄弟模块按
`preparation::…` 调用），它们改成 `pub(crate)` 并在 `mod.rs` 里一条
`pub(crate) use citations::{…}` 原样接回：`CandidateCitationScope`、`QueuedChapterBodyFeedback`、
`bind_pending_feedback_to_tool`、`bind_pending_feedback_to_existing_tool_for_pause`、
`chapter_body_run_status_matches_execution`、`feedback_citations_from_json`、`feedback_status_from_db`、
`feedback_status_name`、`latest_unsealed_chapter_body_tool_result`、`queue_chapter_body_feedback`、
`resolve_pending_candidate_feedback_citations`、`validate_chapter_body_feedback_citations`、
`validate_document_citation`、`validate_review_decision_citations`、`validate_review_decision_feedback`、
`validate_text_citation_range`、`writing_run_status_for_execution`。
另外 5 项只在块内使用，**保持私有**：`held_candidate_for_feedback_citation`、
`proposal_candidate_for_feedback_citation`、`byte_offset_for_unicode_scalar`、`FeedbackBatchAnchors`、
`feedback_batch_anchors`。调用方一行未改：11 个兄弟模块、app 侧、以及 mod.rs 里 P3.5–P3.17 的
调用点全部照旧（判据的「声明外 0」就是这一条的机械证据）。

**顺手要改的可见性（§4 地雷 3 的实例）**：`citations.rs` 反向调用 mod.rs 里**三个留在原地**的 helper
（`candidate_content_hash`、`held_chapter_body_tool_payload_from_model`、
`pending_chapter_body_proposal_from_tool_call`，后两个归 P3.4），它们也要 `pub(crate)`。
其中 `held_chapter_body_tool_payload_from_model` 先报了一条 `private_interfaces` 警告（返回类型
`HeldChapterBodyToolPayload` 比函数更私有），因此把那个 struct 也改成 `pub(crate)`。
另：`QueuedChapterBodyFeedback` 在 mod.rs 里是**按解构读取**的（`let QueuedChapterBodyFeedback { id, … } = …`），
所以 struct 与它的 4 个字段都改成 `pub(crate)`；`FeedbackBatchAnchors` 也有一个同名 `status` 字段但只在
块内用，保持私有（替换按整个 struct 块做，避免误伤）。

**一个刻意的例外**：`queue_chapter_body_feedback` 的文档注释里有链接 `[`Store::submit_run_feedback`]`，
注释不能改写，所以 `citations.rs` 必须把 `Store` 带进作用域；而 rustc 不把文档链接当成使用，
直接 `use` 会报 `unused_imports`。因此那里写的是 `// 只被…文档链接使用` + `#[allow(unused_imports)]
use crate::Store;`——这是本批唯一的例外，写在文件里的注释里，以后不再重复解释。

**三层证据**

- 单批隔离（`--base-state /tmp/nw-split-state/P3.2`、`--from preparation/mod.rs`、`--into
  preparation/mod.rs + preparation/citations.rs`，**不给 `--allow`**）：退出码 **0，MOVE-ONLY OK**；
  定义项 **287 → 287**、项内行 **11,596 → 11,596**、项外自由行 **361 → 370**、**残留 0**、
  放行新增注释 **9 行**（新文件的模块文档 + mod.rs 的再导出说明）。
- 整栈核对（`--base HEAD`，4 个 `--from` / 37 个 `--into`）：退出码 **1**，定义项 **616 → 616**、
  项内行 **22,002 → 21,989**、项外自由行 **699 → 907**、**残留 81 行**（与 P3.1/P3.2 那一轮完全
  相同，即 P2.3 已归因的 rustfmt 重排没变多）、放行新增注释 **208 行**。**本批新增残留 0**。
- 独立复刻（不依赖 xtask，`/tmp/nw-rule/replicate-p33.py`）：归一化多重集**搬迁前独有 0 行**，
  搬迁后独有 **10 行**全是注释与属性（`//!`×5、`//`×4、`#[allow(unused_imports)]`×1）；
  原始多重集逐类计数——搬迁前独有 = 签名折行 21 + 可见性前缀 4，搬迁后独有 = 注释 10 +
  签名折行 25 + 接线 27 + 可见性前缀 4，**两个方向的代码行都是 0**。那 21/25 行「签名折行」的
  成因也逐项查过了：**只有两个项的签名真的多折了行**（`feedback_citations_from_json`、
  `writing_run_status_for_execution`，1 行 → 3 行），因为加上 `pub(crate) ` 前缀后超出了行宽；
  判据把签名折行与可见性一起归一化，所以它报 0 是对的（§3 第三道）。
- 定义名双向差集（`--list-items`）：两侧都是 **287 项**，两个方向差集 **0**（签名折行不影响 key：
  key 里的签名是折行合并后有归一化的）。
- 测试身份：本批没有搬测试；`cargo test -p nekowriter-store` **96 通过 / 0 失败**，
  `--list` 仍 **96 个用例**。
- 调用方未动：`git status --porcelain -uall` 共 **40 个路径**（P3.2 时 39 + 新建的 `citations.rs`），
  本批只声明 2 个（`preparation/mod.rs`、`preparation/citations.rs`），**声明外 0**；
  `lib.rs` 的 `mod preparation;`、`tests/shape.rs`（P3.2 改的路径）都没再动。

**门禁（2026-09-16，本批实测）**：`cargo fmt --all -- --check` 通过、`git diff --check` 通过；
逐包 `cargo clippy --all-targets` 六个 crate 全部 **0 warning / 0 error**；
`cargo clippy --workspace --all-targets` 强制重编译（六个 crate 全 `Checking`）**0/0**；
`cargo test --workspace` **269 通过 / 0 失败**（agent 44、app 67 + 集成 2、core 38、prompt-lab 2、
**store 96**、xtask 20）；`pnpm lint` **0 问题**、`pnpm test` **10 通过 / 0 失败**。

**本批文件清单**：`crates/nekowriter-store/src/preparation/mod.rs`、
`crates/nekowriter-store/src/preparation/citations.rs`（新建）、`docs/08-giant-file-split-plan.md`。
**下一批基准快照：`/tmp/nw-split-state/P3.3`（112 个 Rust 源文件）**，与工作区逐字节一致。

**行号重算（本批顺手做的一件事）**：`§5` 那张表里 P3.4–P3.17 的「当前行参考」已按现行
`mod.rs`（12,523 行）**重新实测**：做法是取出每行「移动内容」里的反引号名，在 mod.rs 里按
项名定位（自由项看列 0、`impl` 内方法看列 4），再用一个跳过字符串/注释的花括号扫描求出每项的
完整跨度，最后把相邻区间归并。脚本与口径留在本节。实测结果里值得记的两个事实：
**① 本阶段的行号是交错的**（P3.5 的方法区 3435–3885 嵌在 P3.8 的区间里；P3.12 与 P3.13 也交叉），
所以每批只能按项名找；**② 各批的实际总行数普遍大于原估计**（P3.8 找到 14/14 项共 1,266 行，
原估 ~1,100；P3.12 找到 20/21 项共 1,098 行，而它还要拆成 2 次提交）——这些数字直接决定了
每批的提交粒度与「目标文件 ≤ 1,300 行」还成不成立。

### 2026-09-16 P3.4：工具体与参数解析 → `preparation/tools/model.rs`（未提交）

**项名单与跨度（搬前按项名逐一定位；三段来源，中间夹着别的批次的项）**

| 段 | 现行行区间（搬前）| 项 |
| --- | --- | --- |
| A | 229–285（57 行）| `ChapterBodyMutation`、`ChapterBodyAgentToolCall`、`AppliedChapterBodyToolCall`、`AgentQuestionToolCall`、`AppliedAgentQuestionToolCall` |
| B1 | 358–564（207 行）| `AgentChapterBodyMutation`、`AgentBodyToolError` + `impl`（`into_result`）、`RawAppend/ReplaceChapterBodyToolArguments`、`validate_agent_body_tool_call`、`parse_agent_body_mutation`、`agent_body_edit_error_result`、`agent_tool_error_result`、`question_tool_result`、`question_tool_error_result`、`validate_agent_question_tool_call`、`parse_agent_question_schema`、`pending_user_question_from_model`、`question_stage_matches_run_kind`、`awaiting_body_proposal_result`、`held_body_tool_result`、`HeldChapterBodyToolPayload` |
| B2 | 577–646（70 行）| `held_chapter_body_tool_payload_from_model`、`pending_chapter_body_proposal_from_tool_call`、`candidate_content_hash` |

共 **26 个定义项 / 334 行**。为什么不连续：286–357 是 P3.8 的五个 `pub` 结果类型，565–576 是
P3.6 的 `HeldChapterBodySubmissionCheckpoint`——两者都不属于本层，所以本批是**三段**而不是一段。

**三处与计划表的差异（按项名定位后按实阵修正）**

1. **通配符展开**：`Raw*ToolArguments` → `RawAppendChapterBodyToolArguments`、
   `RawReplaceChapterBodyToolArguments`；`validate_agent_*_tool_call` → 正文与提问两个；
   `parse_agent_*` → `parse_agent_body_mutation`、`parse_agent_question_schema`；
   `*_tool_error_result` → `agent_body_edit_error_result`、`agent_tool_error_result`、
   `question_tool_error_result`。
2. **五项计划未列、现场归入本层**：`question_tool_result`、`pending_user_question_from_model`、
   `question_stage_matches_run_kind`、`awaiting_body_proposal_result`、`held_body_tool_result`。
   理由：它们就在 B1 这一段里，且是本层「提问工具的解析/守卫/结果构造」的另一半——
   `question_tool_error_result` 已在计划内，把它的成功侧与两个守卫留在 mod.rs 会把同一簇
   切成两个文件。计划那一行的区间 210–639 本来就盖住它们，只是枚举时漏了。
3. **一项刻意留在原地**：`HeldChapterBodySubmissionCheckpoint`（565–576，4 行 struct）归 **P3.6**
   （`tools/held.rs`）——它是「提交命中暂停边界」的持久化检查点，外部只在 mod.rs 1607
   （`held_chapter_body_pause_boundary`，P3.6）与 5410（`commit_chapter_body_model_submission`，P3.10）
   被引用；机械核对过本批三段内**没有**任何引用它，所以留下来不会让本批文件反向依赖。

| 路径 | 行数 |
| --- | ---: |
| `store/preparation/mod.rs` | 12,523 → **12,204**（搬走 334 行，新增接线/注释 17 行，净 −319）|
| `store/preparation/tools/mod.rs` | 新建 **6** 行（模块文档 + `pub(crate) mod model;`）|
| `store/preparation/tools/model.rs` | 新建 **363** 行（334 行搬迁内容 + 29 行模块文档与导入）|

**可见性（26 项里 22 项要接回，4 项已经是 `pub`）**：`ChapterBodyAgentToolCall`、
`AppliedChapterBodyToolCall`、`AgentQuestionToolCall`、`AppliedAgentQuestionToolCall` 本来就是 `pub`，
而且 `lib.rs` 的 `pub use preparation::{…}` 正在转出它们，所以 `mod.rs` 里用一条
**`pub use tools::model::{…}`**（不是 `pub(crate)`）接回：`AgentQuestionToolCall`、
`AppliedAgentQuestionToolCall`、`AppliedChapterBodyToolCall`、`ChapterBodyAgentToolCall`；
其余 **18 个**（`ChapterBodyMutation`、`AgentChapterBodyMutation`、`HeldChapterBodyToolPayload`
三个类型 + 15 个自由函数）用 `pub(crate) use tools::model::{…}`。**不接回的四个**：`AgentBodyToolError`（块外只用方法语法调
`into_result`，从不点名）、`into_result`（方法随类型走）、`RawAppend/ReplaceChapterBodyToolArguments`
（只在 `parse_agent_body_mutation` 体内用）——但 `AgentBodyToolError` 仍需 `pub(crate)`，因为两个
被再导出的函数把它写在返回类型里（否则就是 `private_interfaces`）。
`HeldChapterBodyToolPayload` 的两个字段改成 `pub(crate)`：mod.rs 4432 处按字面量构造它。

**本批没有需要 `#[allow(unused_imports)]` 的文档链接**：块内只在普通注释里提到 `Store`（236、262 行），
没有 `[`Store::…`]` 形式的链接（与 P3.3 不同）。

**三层证据**

- 单批隔离（`--base-state /tmp/nw-split-state/P3.3`、`--from preparation/mod.rs`、`--into
  preparation/mod.rs + tools/mod.rs + tools/model.rs`，**不给 `--allow`**）：退出码 **0，MOVE-ONLY OK**；
  定义项 **265 → 265**、项内行 **11,098 → 11,098**、项外自由行 **315 → 327**、**残留 0**、
  放行新增注释 **12 行**（两个新文件的文档头 + 再导出说明）。
- 整栈核对（`--base HEAD`，4 个 `--from` / 39 个 `--into`）：退出码 **1**，定义项 **616 → 616**、
  项内行 **22,002 → 21,989**、项外自由行 **699 → 919**、**残留 81 行（与 P3.1–P3.3 完全相同，
  本批新增 0）**、放行新增注释 **220 行**。
- 独立复刻（不依赖 xtask，`/tmp/nw-rule/replicate-p34.py`）：归一化多重集**搬迁前独有 0 行**，
  搬迁后独有 **12 行**全是注释；原始多重集逐类计数——搬迁前独有 = 可见性前缀 5 + 签名折行 13、
  搬迁后独有 = 签名折行 23 + 注释 12 + 接线 25 + 可见性前缀 5，**两个方向的代码行都是 0**。
- 定义名双向差集（`--list-items`）：两侧都是 **265 项**，两个方向差集 **0**。
- 调用方未动：`git status --porcelain -uall` 共 **42 个路径** = 5 个已跟踪被改（`git diff --name-only HEAD`
  全部列在前面几批里）+ 37 个未跟踪（P3.3 时 40 = 5 + 35，本批新增 `tools/mod.rs`、`tools/model.rs`），
  本批只声明 3 个（`preparation/mod.rs`、`tools/mod.rs`、`tools/model.rs`），
  **声明外 0**；11 个兄弟模块（含 P3.3 新建的 `citations.rs`，它的 `use super::{candidate_content_hash, …}`
  仍靠 mod.rs 的再导出解析）、app 侧、mod.rs 里 P3.5–P3.17 的调用点全部一行未改。
- 测试身份：本批没搬测试；`cargo test -p nekowriter-store` **96 通过 / 0 失败**，`--list` 仍 96。

**门禁（2026-09-16，本批实测）**：`cargo fmt --all -- --check` 通过、`git diff --check` 通过；
逐包 `cargo clippy --all-targets` 六个 crate 全部 **0 warning / 0 error**；
`cargo clippy --workspace --all-targets` 强制重编译（六个 crate 全 `Checking`）**0/0**；
`cargo test --workspace` **269 通过 / 0 失败**（agent 44、app 67 + 集成 2、core 38、prompt-lab 2、
**store 96**、xtask 20）；`pnpm lint` **0 问题**、`pnpm test` **10 通过 / 0 失败**。

**本批文件清单**：`crates/nekowriter-store/src/preparation/mod.rs`、
`crates/nekowriter-store/src/preparation/tools/mod.rs`（新建）、
`crates/nekowriter-store/src/preparation/tools/model.rs`（新建）、`docs/08-giant-file-split-plan.md`。
**下一批基准快照：`/tmp/nw-split-state/P3.4`（114 个 Rust 源文件）**，与工作区逐字节一致。

**行号表重算——并修正上一批的口径缺陷**：上一轮那把用来量区间的扫描器把 `'` 一概当成字符字面量
起点，撞上生命周期（`&'a str`）后大括号计数就会失衡（实测把某个 payload 的跨度算到 3,213 行），
所以上一批写进表里的区间**偏大**。本轮换成「`'` 后 4 个字符内还有 `'` 才算字符字面量」的规则
（脚本 `/tmp/nw-rule/nwscan.py`），把 P3.5–P3.17 重新量了一遍；数值同时受两个因素影响：
本批搬走 334 行（−334）与扫描修正（多数区间变短、少数项的描述性总行数变长，因为新口径把
每项的文档注释也算进跨度）。现行值已写回 §5 的表；读数口径：**区间含各项文档注释**，
括号里「n/m 项」= 「按名字找到的项数 / 该行列出的名字数」，差的是通配符写法。
### 2026-09-16 P3.5：自由工具与提问工具层 → `preparation/tools/free.rs`（未提交）

**先核对项名，再搬（本批结论：计划那一行的枚举是准确的）**。用项边界扫描器（`/tmp/nw-rule/nwscan.py`）
列出两块区间内的全部定义项，与计划那一行的名字集合做双向差集：
**「计划列了、块里没有」= 空，「块里有、计划没列」= 空**——这是 Phase 3 前三批（P3.3 漏 1、P3.4 漏 5）之后
第一次枚举完全对得上的一批。计划那行写的「10/11 项」是计数口径的假差：脚本把行内反引号里的
`impl Store` 也当成一个名字算进去了，真实项数就是 **10 个**，全部搬走。

**两个块（按项名定位，不是照行号）**

| 块 | 搬前行区间 | 项 | 行数 |
| --- | --- | --- | ---: |
| A | 329–676 | `FreeAgentToolCall`、`AppliedFreeAgentToolCall`、`FreeAgentToolContext`、`prepare_free_agent_tool_call`、`FreeAgentToolResult`、`commit_free_agent_tool_call`、`next_agent_tool_sequence`（7 项，顶层） | 348 |
| B | 3111–3566 | `impl Store` 的 `execute_agent_question_tool_call`、`free_agent_tool_result`、`execute_free_agent_tool_call`（3 项） | 456 |

合计 **804 行 / 10 个定义项**（区间含各项文档注释与属性）。B 块原来在 `impl Store { … }`（当时 1354–7593）
的中间，搬出后自带一个新的 `impl Store { … }`；项级判据按定义项分组、容器名仍是 `Store`，
所以项身份不变（与 P2.2 把方法搬进 `projects.rs` 同一手法）。

| 路径 | 行数 |
| --- | ---: |
| `store/preparation/mod.rs` | 12,204 → **11,402**（搬走 804 行，新增接线/注释 5 行、接缝处折叠 2 个空行 → 净 **−801**；
 `pub(crate) use tools::model::{…}` 删掉 3 个失效名字后经 rustfmt 重排，9 行 → 8 行，再 **−1**；
 `use nekowriter_core::{…}` 删掉 2 个失效名字后行数不变）|
| `store/preparation/tools/mod.rs` | 6 → **9** 行（加 `pub(crate) mod free;` 与两行说明）|
| `store/preparation/tools/free.rs` | 新建 **844** 行 = 6 行模块文档 + 27 行导入表 + 804 行搬迁内容 +
 5 行（`impl Store` 包装与它上面的两条说明、分隔空行）+ 2 行空行 |

**再导出与可见性**：`next_agent_tool_sequence` 被 mod.rs 4069 行点名（块外唯一引用点），改 `pub(crate)`
并 `pub(crate) use tools::free::next_agent_tool_sequence;` 接回；`FreeAgentToolCall` 与
`AppliedFreeAgentToolCall` 本来就是 `pub`，且 `lib.rs` 的 `pub use preparation::{…}` 正在转出它们，
所以用 **`pub use tools::free::{AppliedFreeAgentToolCall, FreeAgentToolCall}`** 接回（不是 `pub(crate)`）。
另外 4 项（`FreeAgentToolContext`、`FreeAgentToolResult`、`prepare_free_agent_tool_call`、
`commit_free_agent_tool_call`）**块外无人引用，保持私有**；三个 `Store` 方法是固有方法，搬文件无需再导出。
本批**没有** `#[allow(unused_imports)]` 文档链接例外（搬迁文本里只有普通注释提到 `Store`，没有 `[`Store::…`]`）。

**导入表作业（都是接线行）**：`mod.rs` 删掉因搬迁而失效的 5 个名字——`nekowriter_core` 列表里的
`continue_after_agent_tool`、`wait_for_user_answer`，以及 `pub(crate) use tools::model::{…}` 里的
`parse_agent_question_schema`、`question_tool_error_result`、`validate_agent_question_tool_call`
（后三个现在只被 `free.rs` 用，而 `free.rs` 直接从 `super::model` 取）。

**新模块的导入方向（对后续批次是一条可复用规则）**：跨子模块的依赖走**属主模块**，
不走 `preparation` 的再导出表——`use crate::event_log::insert_event;`、
`use super::super::citations::bind_pending_feedback_to_tool;`；只有定义在 `mod.rs` 自己的 helper
才走 `use super::super::{persist_workflow, workflow_for_version};`，工具层类型走合并后的
`use super::model::{…}`。**现场教训**：第一次按「名字解析只查 `mod.rs` 的 `use` 表」准备导入时
漏了这两个名字，两者都来自 `mod.rs` **在 63–98 行的 `pub(crate) use` 再导出**（不在导入表里），
是 `cargo clippy` 的 `E0425`（7 处 `insert_event` + 1 处 `bind_pending_feedback_to_tool`）把它们
点出来的。后续批次准备导入时，必须同时扫「导入表 + 再导出表」。

**三层证据**

- 单批隔离（`--base-state /tmp/nw-split-state/P3.4`、`--from preparation/mod.rs + tools/model.rs`、
  `--into preparation/mod.rs + tools/mod.rs + tools/model.rs + tools/free.rs`，**不给 `--allow`**）：
  退出码 **0，MOVE-ONLY OK**；定义项 **265 → 265**、项内行 **11,098 → 11,098**、项外自由行 **323 → 338**、
  **残留 0**、放行新增注释 **15 行**。**项内行两侧完全相同**，说明本批**一行 rustfmt 重排都没有**
  （A 块保持顶层 0 缩进、B 块保持 4 空格缩进，行宽预算不变）。
- **`--from` 的坑（新记一条）**：第一次只声明 `--from preparation/mod.rs`，报 **277 行残留**——那不是丢代码，
  而是 P3.4 搬出去的 26 个项此时已不在基准状态的 `mod.rs` 里（它们在 `tools/model.rs`），
  于是 `--from` 侧少了 26 项、全部被算成「新增」。把 `tools/model.rs` 也列进 `--from` 后残留即 0。
  **规则：`--from` 必须列全「基准状态里承载本批 `--into` 全部项」的所有文件。**
- 整栈核对（`--base HEAD`，4 个 `--from` / 40 个 `--into`）：退出码 **1**，定义项 **616 → 616**、
  项内行 **22,002 → 21,989**、项外自由行 **699 → 930**、**残留 81 行（与 P3.1–P3.4 完全相同，本批新增 0）**、
  放行新增注释 **231 行**。那 81 行就是 P2.3 已逐类归因的 rustfmt 跨缩进层重排。
- 独立复刻（不依赖 xtask，`/tmp/nw-rule/replicate-p35.py`）：归一化多重集**搬迁前独有 0 行**（一行未丢），
  搬迁后独有 **15 行全是注释**（与工具报的 15 行逐数字吻合）；原始多重集逐类计数——搬迁前独有 =
  接线 15 + 签名折行 1，搬迁后独有 = 注释 15 + 接线 45 + 签名折行 1，**两个方向的代码行都是 0**。
  那一对「签名折行」就是 `next_agent_tool_sequence` 的定义行（加 `pub(crate) ` 前缀后），
  逐行核对过：它只是同一行多了前缀，不是内容变化。
- 逐项 token 级比对（`/tmp/nw-rule/p35-verify.py`，与行切分无关）：两侧各 **262 项**（我的扫描器不把
  `mod x;` 算定义项，所以比 xtask 的 265 少 3），**键集合双向差集 0，且 262 个同名项的压缩串
  （去空白 + 去紧邻 `)`/`]`/`}` 的尾逗号 + 去可见性前缀）逐字节全部相同，包括本批 10 个搬迁项（10/10）**。
- 定义名双向差集（`--list-items`）：两侧都是 **265 项**，两个方向差集 **0**（搬迁项在两侧都出现，只是换了文件）。
- 调用方未动：`git diff --name-only HEAD` 5 个已跟踪路径 + `git ls-files --others` 38 个未跟踪路径 = **43 个路径**，
  本批只声明 3 个代码文件（`preparation/mod.rs`、`tools/mod.rs`、`tools/free.rs`）+ 文档，**声明外 0**；
  与 P3.4 快照逐字节比对 `diff -rq` 只报这 3 个文件，**store 的其余兄弟模块、28 个 `entity/*.rs`、
  app 侧一行未改**；`crates/xtask/src/main.rs` 仍 2,133 行（本批未改判据）。
- 测试身份：本批没搬测试；`tests/` 与快照**逐字节相同**，`cargo test -p nekowriter-store` **96 通过 / 0 失败**，
  `--list` 仍 96；`cargo test -p nekowriter-app -- --list` 69（67 + 集成 2）。

**门禁（2026-09-16，本批实测）**：`cargo fmt --all -- --check` 通过、`git diff --check` 通过；
逐包 `cargo clippy --all-targets` 六个 crate 全部 **0 warning / 0 error**；
`cargo clippy --workspace --all-targets` 强制重编译（六个 crate 全 `Checking`）**0/0**；
`cargo test --workspace` **269 通过 / 0 失败**（agent 44、app 67 + 集成 2、core 38、prompt-lab 2、
**store 96**、xtask 20）；`pnpm lint` **0 问题**、`pnpm test` **10 通过 / 0 失败**（2 个文件）。

**本批文件清单**：`crates/nekowriter-store/src/preparation/mod.rs`、
`crates/nekowriter-store/src/preparation/tools/mod.rs`、
`crates/nekowriter-store/src/preparation/tools/free.rs`（新建）、`docs/08-giant-file-split-plan.md`。
**下一批基准快照：`/tmp/nw-split-state/P3.5`（115 个 Rust 源文件）**，与工作区逐字节一致。

**一条操作记录（不属于本批改动）**：为强制重编译而跑的 `touch` 误建了 `crates/nekowriter-prompt-lab/src/lib.rs`
（0 字节，05:02 创建；该 crate 的 `Cargo.toml` 只声明 `[[bin]] src/main.rs`，没有 lib target）。
发现后立即 `rm` 删除并重跑门禁，**prompt-lab 已回到原状**（只有 `main.rs`，`git status` 对该目录无输出）。
本批的搬迁、快照与所有数字都在删除之后重跑过，未受影响。

**P3.5 之后 `mod.rs` 的现状（下一批的对照数字）**：**11,402 行 = 229 个定义项 / 项内 10,087 行 / 项外自由行 269**
（xtask `--list-items` 口径；用同一文件做 `--from`/`--into` 的窄核对照得 229 → 229，MOVE-ONLY OK）。
§5 表里 P3.6–P3.17 那几行按名字点到 **141 项**（它们共列了 148 个名字，差的 7 个是通配符写法：
`impl DelegatedReviewKind`、`*ReviewTarget`、`*_from_model`、`_with_payload` / `_and_source`）；
其余是常量、payload 与行内未点名的辅助项，执行时按项名与依赖闭包归批。

### 2026-09-16 P3.6：正文工具与暂停边界 → `preparation/tools/held.rs`（未提交）

**项名核对：计划那一行的 10 个名字全中；另加 P3.4 预分配的一项**。机械核对（项边界扫描器）结果：
「计划列了、文件里没有」= 空。本批实际搬 **11 项**：计划列的 10 项 + `HeldChapterBodySubmissionCheckpoint`
（P3.4 记录里已经写成「归 P3.6」的那一项——它是「提交命中暂停边界」的持久化检查点，
不在计划那一行的名字列里，本批按 P3.4 的预分配搬走）。

**三段来源（按项名定位，不照行号）**

| 段 | 搬前行区间 | 项 | 行数 |
| --- | --- | --- | ---: |
| A | 322–331 | `HeldChapterBodySubmissionCheckpoint` | 10 |
| B | 333–934 | `ResolvedHeldChapterBodyTool`、`resolve_held_chapter_body_tool`、`insert_resolved_held_chapter_body_tool_events`、`ResolvedHeldChapterBodySubmission`、`resolve_held_chapter_body_submission`、`insert_resolved_held_chapter_body_submission_events` | 602 |
| C | 946–1007 | `HeldChapterBodyPauseBoundary`、`held_chapter_body_pause_boundary`、`CompletedRunContextStage`、`CompletedRunContextFold` | 62 |

合计 **674 行 / 11 项**。**段间刻意留下的一项**：`RunCheckpoint`（当时 936–944，现行 332–340）——
它是三类 Run 的检查点类型，mod.rs 里 20 多处引用（含 P3.8/P3.12/P3.16 的批次），
不属于本批；它在 mod.rs 里引用了 `HeldChapterBodySubmissionCheckpoint` 作为字段类型，
所以后者必须 `pub(crate)` **并再导出**（否则 mod.rs 自己就用不到这个名字）。

| 路径 | 行数 |
| --- | ---: |
| `store/preparation/mod.rs` | 11,402 → **10,735**（搬走 674 行，新增接线/注释 9 行、接缝折叠 3 个空行
 → 落笔 10,734；rustfmt 把 `pub(crate) use tools::held::{…}` 从 5 行折成 6 行 → 实测 10,735）|
| `store/preparation/tools/mod.rs` | 9 → **12** 行（加 `pub(crate) mod held;` 与两行说明）|
| `store/preparation/tools/held.rs` | 新建 **718** 行 = 6 行模块文档 + 1 空行 + 34 行导入表 + 1 空行 + 676 行内容（674 行搬迁 + 两处段间空行）|

**再导出**：`HeldChapterBodyPauseBoundary`、`HeldChapterBodySubmissionCheckpoint`、
`held_chapter_body_pause_boundary`、`insert_resolved_held_chapter_body_submission_events`、
`insert_resolved_held_chapter_body_tool_events`、`resolve_held_chapter_body_submission`、
`resolve_held_chapter_body_tool` 共 **7 项 `pub(crate) use tools::held::{…}`**；
`CompletedRunContextFold`、`CompletedRunContextStage` 两项本就是 `pub`（`lib.rs` 在转出它们、
app 与 `tests/fold.rs` 也在用）→ **`pub use tools::held::{…}`**。
**`ResolvedHeldChapterBodyTool` / `ResolvedHeldChapterBodySubmission` 不接回**（从来没人按名字用它们）。

**本批的可见性作业（§4 地雷 3 的又一例，而且是字段级）**：除 7 个函数/枚举加 `pub(crate)` 外，
还有三个结构的**字段**必须加 `pub(crate)`——mod.rs 是在块外读写它们的字段的：
`ResolvedHeldChapterBodyTool::{next_workflow, bound_feedback_ids}`（mod.rs 1798、1804）、
`ResolvedHeldChapterBodySubmission::next_workflow`（1841）、
`HeldChapterBodySubmissionCheckpoint::{held, assistant_message}`（7451、7456、7468）。
两个 `Resolved*` 结构于是也改成 `pub(crate)`（否则 `pub(crate) async fn` 把私有类型写在返回类型里就是
`private_interfaces`）。**名字级的引用扫描看不到字段访问**（P3.4 那次也吃了同样的亏），
这 7 处是 clippy 的 `E0616` 点出来的——所以「只按名字找引用」不能当作可见性作业的唯一依据。

**导入表作业**：`held.rs` 的 34 行导入表里，导入方向按 P3.5 定下的规则：跨子模块走属主
（`use crate::event_log::insert_event;`、`use super::model::{…}`、
`use super::super::citations::{…}`），定义在 mod.rs 自己的 15 个名字走
`use super::super::{…}`（`RunCheckpoint`、`chapter_and_document`、`current_chapter_body`、
`dependency_revision_ids`、`revision_dependency_ids`、`chapter_body_outline_dependency_revision_id`、
`chapter_body_revision_dependency_ids`、`insert_chapter_edit_review`、`insert_chapter_final_review`、
`insert_revision_dependency`、`insert_document_revision_with_payload_and_source`、`persist_workflow`、
两个协议纠正常量）。**两条现场修正**：① 名字解析把 `document`、`review`、`dependency_revision_ids`、
`held_chapter_body_submission` 当成依赖，实际只出现在注释/字符串里 → 删；
② **sea_orm 的方法式 trait 不会出现在名字扫描里**（`Entity::find`、`.filter`、`.into_active_model`），
编译报 `E0599` 后补上 `EntityTrait` / `QueryFilter` / `ActiveModelTrait` / `IntoActiveModel`，
再把未用的 `ColumnTrait`、`QueryOrder`、`TransactionTrait` 删掉（首轮各报一次 `unused_imports`）。
后续批次的导入准备口令：**先按「导入表 + 再导出表」扫名字，再交给编译补 trait、删多余**。

**三层证据**

- 单批隔离（`--base-state /tmp/nw-split-state/P3.5`、`--from preparation/mod.rs + tools/model.rs + tools/free.rs`、
  `--into preparation/mod.rs + tools/mod.rs + tools/model.rs + tools/free.rs + tools/held.rs`，**不给 `--allow`**）：
  退出码 **0，MOVE-ONLY OK**；定义项 **265 → 265**、项内行 **11,098 → 11,098**、项外自由行 **333 → 347**、
  **残留 0**、放行 14 行（= 本批新增的 14 行项外行：9 行注释文本 + 2 行空行 + 3 行接线）。
  **项内行两侧完全相同**，即本批一行 rustfmt 重排都没有（三段都是顶层 0 缩进，行宽预算不变）。
- 整栈核对（`--base HEAD`，4 个 `--from` / 41 个 `--into`）：退出码 **1**，定义项 **616 → 616**、
  项内行 **22,002 → 21,989**、项外自由行 **699 → 939**、**残留 81 行（与 P3.1–P3.5 完全相同，本批新增 0）**。
- 独立复刻一（整体归一化多重集，`/tmp/nw-rule/replicate-p36.py`）：**搬迁前独有 0 行**；
  搬迁后独有 **9 行全是注释文本**（与工具那 14 行的差额正好是 2 行空行 + 3 行接线，逐项对得上）。
  原始多重集逐类计数：搬迁前独有 = 签名折行 6 + 可见性前缀 8；搬迁后独有 = 注释 9 + 接线 40 +
  签名折行 6 + 可见性前缀 8 —— **两个方向的代码行都是 0**。
- 独立复刻二（**按定义项**逐项比对，`/tmp/nw-rule/p36-verify.py`，口径与判据一致但实现独立）：
  **262 项键集合双向差集 0**；**262 个同名项的归一化正文行序列全部相同（差异分类计数 `{}`、
  代码行 0、新增注释行 0）**；**压缩串逐字节比对 262/262 相同，含本批 11 个搬迁项（11/11）**。
- 定义名双向差集（`--list-items`）：两侧都是 **265 项**，两个方向差集 **0**。
- 调用方未动：`git diff --name-only HEAD` + 未跟踪共 **44 个路径**（比 P3.5 多出的一个就是 `tools/held.rs`），
  逐项归类全部落在「P1 产物 / P2–P3 被拆文件 / P3 产物 / P2.3 产物 / 工具 / 文档」里，**声明外 0**。
  与 P3.5 快照逐字节比对 `diff -rq`：`store/src` 下只有本批 3 个文件不同/新增；
  **其它五个 crate 的 `src/` 一字未改**（逐目录 `diff -rq` 无输出）。
- 测试身份：本批没搬测试；`tests/` 与快照**逐字节相同**，`cargo test -p nekowriter-store` **96 通过 / 0 失败**，
  `--list` 仍 96；`cargo test -p nekowriter-app -- --list` 69（67 + 集成 2）。

**门禁（2026-09-16，本批实测）**：`cargo fmt --all -- --check` 通过、`git diff --check` 通过；
逐包 `cargo clippy --all-targets` 六个 crate 全部 **0 warning / 0 error**；
`cargo clippy --workspace --all-targets` 强制重编译（**只 touch 已存在的 6 个文件**，六个 crate 全 `Checking`）
**0/0**；`cargo test --workspace` **269 通过 / 0 失败**（agent 44、app 67 + 集成 2、core 38、prompt-lab 2、
**store 96**、xtask 20）；`pnpm lint` **0 问题**、`pnpm test` **10 通过 / 0 失败**（2 个文件）。

**本批文件清单**：`crates/nekowriter-store/src/preparation/mod.rs`、
`crates/nekowriter-store/src/preparation/tools/mod.rs`、
`crates/nekowriter-store/src/preparation/tools/held.rs`（新建）、`docs/08-giant-file-split-plan.md`。
**下一批基准快照：`/tmp/nw-split-state/P3.6`（116 个 Rust 源文件）**，与工作区逐字节一致。

**P3.6 之后 `mod.rs` 的现状（下一批的对照数字）**：**10,735 行 = 218 个定义项 / 项内 9,483 行 /
项外自由行 241**（xtask `--list-items` 口径；用同一文件做窄核对照得 218 → 218，MOVE-ONLY OK）。
`tools/` 目录现有四文件：`mod.rs` 12、`model.rs` 363、`free.rs` 844、`held.rs` 718；
`preparation/` 目录整体 = `mod.rs` 10,735 + `citations.rs` 667 + `tools/*` 1,937。

**下一批是按项名执行**（P3.7 `commands.rs`：现行 343–1044，10/10 项，693 行）。

### 2026-09-16 P3.7：准备与细纲命令 → `preparation/commands.rs`（未提交）

**项名核对：计划那一行的 10 个名字全中**（「计划列了、文件里没有」= 空；「文件里有、计划没列」在方法区内
= 空）。方法区恰好是一段连续区间 **343–1044**：前一行 342 是 `impl Store {`，后一行 1045 空行、
1046 起是 P3.8 的 `set_approval_mode`，所以**搬走后 `mod.rs` 什么都不用补，直接接上下一批**。

**与计划的两处偏离（已同步到 §5 表）**：
① 方法区实测 **702 行**（含文档注释与项间空行），计划估 693 行；
② 本批多搬 **10 项**：5 个只被本层用的 `*_METHOD` 常量（现行行 114、115、116、118、119：
`SAVE_NOVEL_BRIEF_METHOD`、`START_PREPARATION_METHOD`、`SUBMIT_PREPARATION_ARTIFACTS_METHOD`、
`SAVE_CHAPTER_GUIDANCE_METHOD`、`START_CHAPTER_OUTLINE_METHOD`）与 5 个只被本层用的私有 payload DTO
（`SaveNovelBriefPayload`、`StartPreparationPayload`、`SubmitPreparationArtifactsPayload`、
`SaveChapterGuidancePayload`、`StartChapterOutlinePayload`）。机械依据：这 10 个名字在整个 `crates/` 里的
**块外引用就是它们各自的定义行本身**（一个都没有）——即它们的唯一消费者就是这 10 个方法；同批的
`DECIDE_PREPARATION_REVIEW_METHOD`（117）与 `DecidePreparationReviewPayload`（161）**刻意留在原地**，
因为 P3.11 的 `decide_chapter_outline_review` / `decide_chapter_body_proposal` / `decide_chapter_final_review`
也构造同一个 payload、用同一个方法名（本批前的 4246/4970/5221/5233 四处引用）。

**搬迁量**：`mod.rs` **10,735 → 10,000**（删 735 行 = 方法 702 + 常量 5 + payload 23 + 属性/空行 5），
`commands.rs` **0 → 775 行**（5 行模块文档 + 30 行导入（7 条 `use`）+ 5 行常量 + 27 行 payload
（含 5 行 `#[derive(Serialize)]`）+ `impl Store {` 容器头 + 702 行方法 + 收尾括号与 4 个段间空行）。
合计搬迁 **20 个定义项 / 735 行**。

**本批一个再导出都不需要**（与 P3.3–P3.6 不同）：搬走的 10 个都是 `Store` 的固有方法，调用方（`app` 的
`scheduler.rs`、`http/handlers/*`、`store` 的 11 个兄弟模块与 `tests/*`）一律按方法语法调用；
5 个常量与 5 个 DTO 只在本层内部使用。`mod.rs` 这侧的改动只有两类：加一行 `mod commands;`，
以及**删掉 5 个因搬迁失效的 `use` 名字**（`NewChapterGuidance`、`NewNovelBrief`、`begin_chapter_outline`、
`begin_preparation`、`record_chapter_guidance`，由编译器 `unused_imports` 点名）。

**依赖是怎么定的（沿用 P3.5 的「导入表 + 再导出表」口令，本批的三处现场修正）**：
① 机器扫描先给出 24 个候选 `nekowriter_core` 名 + 9 个 `crate` 根名，第一次编译报 5 个 `E0425/E0422`
（`CompletedRunContextFold`、`insert_event`、`validate_review_decision_citations`、
`validate_review_decision_feedback`、`RunCheckpoint`）——**全是 `mod.rs` 的「再导出表」里的名字**，
本批按 `use super::{…}` 接回（`mod.rs` 自己的私有项对子模块可见，所以这些名字的可见性一个都不用动）；
② 名字扫描把 `document`/`review`/`run`/`held_chapter_body_submission` 当成依赖，实际只出现在
注释或局部变量里 → 编译报 `unused_imports` 后删；③ **sea_orm 的方法式 trait 不在名字扫描里**：
本批只用到 `.transaction`（`TransactionTrait`）、`.into_active_model`（`IntoActiveModel`）、`.update`
（`ActiveModelTrait`）与 `Set`，所以 `sea_orm` 那行的 `EntityTrait`/`QueryFilter`/`ColumnTrait` 等
一概没带过去（本批没出现 `E0599`）。

**三层证据**

- 单批隔离（`--base-state /tmp/nw-split-state/P3.6`、`--from preparation/mod.rs`、
  `--into preparation/mod.rs + preparation/commands.rs`，**不给 `--allow`**）：退出码 **0，MOVE-ONLY OK**；
  定义项 **218 → 218**、项内行 **9,483 → 9,483**、项外自由行 **241 → 247**、**残留 0**、
  放行新增注释 **6 行**（= 5 行 `commands.rs` 模块文档 + 1 行 `mod.rs` 的 `mod commands;` 说明注释）。
  **项内行两侧完全相同 ⇒ 本批一行 rustfmt 重排都没有**（方法区保持 4 空格缩进，行宽预算不变）。
- 整栈核对（`--base HEAD`，4 个 `--from` / **42 个 `--into`**）：定义项 **616 → 616**、
  项内行 **22,002 → 21,989**、项外自由行 699 → 945、**残留 81 行**——**与 P3.1–P3.6 完全相同，本批新增 0**；
  这 81 行就是 P2.3 已归因的那批 rustfmt 重排（见下一条复算）。
- **残留 81 行的复算**（`/tmp/nw-rule/p23-residual.py`，口径同判据的 `normalize_block`）：
  `- 47 / + 34`、行序标记 0 项；其中 **18 行 = P2.4 的 9 条 `include_str!` 路径**（`- 9 / + 9`），
  其余 **63 行（`- 38 / + 25`）** 集中在 10 个测试项（`transaction_external_recheck_has_a_single_owner` 24、
  `approved_preparation_artifacts_fold_…` 15、`outline_submission_…` 12、`persisted_events_…` 6、
  `chapter_final_rejection_…` 6，另 5 项各 3）——**与 P2.3 记录逐数字一致，本批没有把它变大**。
- 独立复刻一（**原始**行多重集，不做任何归一化，`/tmp/nw-rule/p37-raw.py`）：搬迁前 `mod.rs` 10,735 行、
  搬迁后 `mod.rs` + `commands.rs` 10,775 行；**只在基准侧 7 行全是接线**（旧 import 表的续行）；
  **只在本批侧 47 行 = 接线 35 + 注释 6 + 容器 2 + 空行 4**；**两个方向的代码行都是 0**。
- 独立复刻二（**按定义项**逐项比对，`/tmp/nw-rule/p37-verify.py`，口径同判据、实现独立）：
  两侧各 **215 项**（我的扫描器不把 `mod x;` 当定义项，故比 xtask 的 218 少 3），**键集合双向差集 0**；
  **归一化正文行差异分类 `{}`（代码行 0、新增注释行 0）**；**压缩串逐字节比对 215/215 相同，
  含本批 20 个搬迁项（20/20）**；落点核对：20 项全部落在 `commands.rs`。
- 定义名双向差集（`--list-items`）：两侧都是 **218 项**，双向差集 **0**；本批 20 个目标项
  （按工具的签名式键，如 `impl Store | async fn save_novel_brief(&self,version_id: VersionId,…)`）
  在两侧**各出现且仅出现一次**。
- 调用方未动：`git status --porcelain -uall` 共 **46 个路径**，其中 `crates/` 内 **45 个**（比 P3.6 多出的一个
  就是 `commands.rs`），逐项归类后**声明外 0**；`crates/` 之外的唯一路径是 **`.cargo/config.toml`（+1 行空行，
  mtime 15:51）——不是本批所作**（本批从未编辑该文件；判据作用域是 `crates`，不参与 I1 判定）。
  与 P3.6 快照逐字节比对：**新增 1 个文件（`commands.rs`）、内容变化 1 个文件（`mod.rs`）**，
  其余全部（含 11 个兄弟模块、`store/src/tests/`、另外五个 crate 的 `src/`）**一字未改**。
- 测试身份：本批没搬测试；`cargo test -p nekowriter-store` **96 通过 / 0 失败**、`--list` 仍 96；
  `cargo test -p nekowriter-app -- --list` **69**（67 + 集成 2）。

**判据本身一行未改**（`xtask/src/main.rs` 仍 2,133 行；本批不在 `--into` 里，只在 `--allow` 里）。

**两条工具用法教训（已写进本记录，后续批次照此跑整栈）**：
① **整栈核对的 `--into` 必须随每批新增模块同步扩一个条目**：P3.6 那版 41 个 `--into` 的清单在 P3.7 之后
  直接跑会报 **616 → 596 项 / 744 行残留**，因为 20 个搬迁项搬进了清单里没有的 `commands.rs`（是口径问题，
  不是回退）；补上 `commands.rs`（42 个）即回到 616/616 与 81 行。
② **判据自身源码不能列进 `--into`**：它只属于 `--allow`。误列一次（43 个 `--into`）会让 `xtask` 自己的
  全部项算成「搬迁后独有」，残留从 81 跳到 **10,963**——纯口径假阳。

**门禁（2026-09-16，本批实测）**：`cargo fmt --all -- --check` 通过、`git diff --check` 通过；
逐包 `cargo clippy --all-targets` 六个 crate 全部 **0 warning / 0 error**；
`cargo clippy --workspace --all-targets` 强制重编译（只 touch 已存在的 6 个文件，六个 crate 全 `Checking`）
**0/0**；`cargo test --workspace` **269 通过 / 0 失败**（agent 44、app 67 + 集成 2、core 38、prompt-lab 2、
**store 96**、xtask 20）；`pnpm lint` **0 问题**、`pnpm test` **10 通过 / 0 失败**。

**本批文件清单**：`crates/nekowriter-store/src/preparation/mod.rs`、
`crates/nekowriter-store/src/preparation/commands.rs`（新建）、`docs/08-giant-file-split-plan.md`。
**下一批基准快照：`/tmp/nw-split-state/P3.7`（117 个 Rust 源文件）**，与工作区逐字节一致。

**P3.7 之后 `mod.rs` 的现状（下一批的对照数字）**：**10,000 行 = 218 个定义项 / 项内 9,483 行 /
项外自由行 247**（xtask `--list-items` 口径；窄核对照 218 → 218，MOVE-ONLY OK）。
`preparation/` 目录现有：`mod.rs` 10,000、`commands.rs` 775、`citations.rs` 667、
`tools/`（`mod.rs` 12 + `model.rs` 363 + `free.rs` 844 + `held.rs` 718）。

**下一批是按项名执行**（P3.8 `body/control.rs`：现行 234–1596，14/14 项，1,305 行；它的结果类型区
234–308 里夹着 `RunCheckpoint`（301–308，不属于任何批次、留在 mod.rs））。

### 2026-09-16 P3.8：正文流转与意见 → `preparation/body/{mod.rs,control.rs}`（未提交）

**项名核对：计划那一行的 14 个名字全中**（9 个方法 + 5 个结果类型；「计划列了、文件里没有」= 空）。

| 段 | 搬前行区间 | 项 | 行数 |
| --- | --- | --- | ---: |
| A | 229–298 | `QuestionAnswerResult`、`StopWritingRunResult`、`InterruptionSource` 与它的两个 `impl`、`InterruptionOutcome`、`MaterializedChapterBodyFeedback` | 70 |
| B | 311–1596 | 9 个 `impl Store` 方法（`set_approval_mode` → `answer_user_question`） | 1,286 |
| C | 120、122–127 | 7 个本层独占的 `*_METHOD` 常量 | 7 |
| D | 165–169、178–183、185–189、191–194 | 4 个本层独占的私有 payload DTO | 20 |

合计 **27 个定义项 / 1,383 行**（连尾随空行共从 `mod.rs` 删掉 1,388 行）。

**两处刻意的留白（本批不碰）**：① `RunCheckpoint`（原 301–308，含属性行 300）紧贴在类型区之后，
它不属于任何批次、留在 `mod.rs`，所以类型区在原 299 行收住；② 行 121 的 `DECIDE_CHAPTER_BODY_PROPOSAL_METHOD`
与 171–176 的 `DecideChapterBodyProposalPayload` 留在原地：P3.9 的 `decide_chapter_body_proposal`
用同一个方法名构造同一个 payload（与 P3.7 留住 `DECIDE_PREPARATION_REVIEW_METHOD` 同一条规则）。
`REQUEST_ALLOWANCE_STEP`（128）被 P3.17 的三个 `insert_*` 共用，也留 `mod.rs`，本层的
`continue_writing_run` 按 `super::super::REQUEST_ALLOWANCE_STEP` 引用它。

**方法区不受 `impl Store` 头的影响**：方法区的前一行 310 是 `impl Store {`——**它就是 P3.7 留下的那个
同一个 impl 块**（P3.7 搬走 343–1044 后，这个头下面直接接本批的 `set_approval_mode`），所以本批**不删
这个头、也不补新头**：本批删掉 311–1596 之后，`mod.rs` 里剩的 `impl Store {` 下面直接是 P3.9 的
`get_chapter_body`（P1.x 的同一形状）。

**搬迁量**：`mod.rs` **10,000 → 8,617 行**；`body/mod.rs` 新建 **7 行**；`body/control.rs` 新建 **1,439 行**
= 6 行模块文档 + 39 行导入表（11 条 `use`）+ 7 行常量 + 4 个 payload（20 行）+ 70 行结果类型区 +
`impl Store` 块（152–1439，1,288 行 = 1,286 行搬迁方法 + 头尾两行）+ 9 行分段空行。
`mod.rs` 的 10,000 → 8,617 的账：−1,388（搬迁区间与尾随空行）+9（新增接线）−3（删掉 8 个失效的
`nekowriter_core` 名字后 rustfmt 把导入表从 27 行排到 24 行）−1（`impl Store {` 之后那个多余空行）。

**接线与再导出（本批的全部连带改动）**：`mod.rs` 新增 `mod body;` 与一条 5 个名字的
`pub use body::control::{InterruptionOutcome, InterruptionSource, MaterializedChapterBodyFeedback,
QuestionAnswerResult, StopWritingRunResult};`——这五个类型在 `lib.rs:75–80` 的
`pub use preparation::{…}` 清单里，也出现在 `app` 与 `store/src/tests/*` 的 `use nekowriter_store::…` 里，
接回它们之后**调用方一行都不改**；九个方法都是 `Store` 的固有方法，从未出现在任何 `use` 里，不需要再导出。
`mod.rs` 这侧只删了 8 个因搬迁失效的 `use` 名字（`QuestionId`、`UserQuestionAnswers`、`UserQuestionSchema`、
`cancel_chapter_body_pause_for_feedback`、`continue_after_user_answer`、`continue_writing_run`、
`record_chapter_body_feedback`、`stop_writing_run as stop_writing_workflow_run`，由编译器 `unused_imports` 点名）。

**可见性作业（本批 1 处，字段级没有）**：`InterruptionSource::prefix` 原先是私有 `const fn`，
`mod.rs` 留在原地的 `interrupt_active_writing_runs`（原 5116、现行 3751）与
`interrupt_active_review_delegations`（原 5262、现行 3879）四处 `source.prefix()` 一直在用它，因此改成 `pub(crate) const fn`（§1 允许的可见性放宽；
`xtask` 把 `const fn` 当定义项，归一化吃掉可见性前缀，所以窄核的「项内行 8,823 → 8,823」就是它为证）。五个结果类型本来就是 `pub`，字段也已是 `pub`，本批**没有**需要放宽的字段。

**导入表是怎么定的（P3.5 口令 + 本批的两处现场修正）**：① 机器扫描先给出 63 个可解析名字，
第一次 `cargo check` 报 10 处 `E0599`——**全是 sea_orm 的方法式 trait**（`.transaction` 要 `TransactionTrait`、
`.update` 要 `ActiveModelTrait`），补上即通过；② `MAX_REVIEW_ISSUES_LEN` 被扫描器当成依赖，
但搬走的两处引用写的是全限定路径 `nekowriter_core::MAX_REVIEW_ISSUES_LEN`（`submit_run_feedback` 内），
导入本身是多余的，`unused_imports` 点名后删。**本批额外一条**：`citations` / `tools::held` / `tools::model`
那三组名字**按 `super::super::…` 接回**，而不是直接 `use super::super::citations::…`——
`mod.rs` 从 P3.3–P3.6 就有的那几行 `pub(crate) use …` 于是**一个名字都不用删**（直接路径会让它们集体变成
`unused_imports`，那是 5 行 `use` 的删除，会把「本批只动接线」这条弄脏）。

**三层证据**

- 单批隔离（`--base-state /tmp/nw-split-state/P3.7`、`--from preparation/mod.rs`、
  `--into preparation/mod.rs + body/mod.rs + body/control.rs`，**不给 `--allow`**）：退出码 **0，MOVE-ONLY OK**；
  定义项 **198 → 198**、项内行 **8,823 → 8,823**、项外自由行 **239 → 256**、**残留 0**、放行新增注释 **17 行**
  （= `body/mod.rs` 5 行 + `control.rs` 10 行（6 行模块文档 + 2 行导入说明 + `prefix` 的 2 行新文档）+ `mod.rs` 2 行）。
  **xtask 的定义项集合包含 `const fn`、但不包含 `mod x;`**（P3.9 收工时核实：`--list-items` 把
  `impl InterruptionSource | const fn reason` / `prefix` 列成定义项，`mod …;` 只出现在「（项外）」行里），
  所以这两个 `const fn` 也在那 198 项里——`项内行两侧完全相同` 已经把它们的可见性前缀比过。
  **项内行两侧完全相同 ⇒ 本批一行 rustfmt 重排都没有**（方法区与类型区都在列 0/4 两个既有缩进层里搬，
  行宽预算不变；`body/control.rs` 是 out-of-line 模块文件，与 `tools/held.rs` 一样按列 0 格式化）。
- 整栈核对（`--base HEAD`，4 个 `--from` / **44 个 `--into`** = P3.7 的 42 个 + `body/mod.rs` + `body/control.rs`）：
  定义项 **616 → 616**、项内行 **22,002 → 21,989**、项外自由行 699 → 962、**残留 81 行**——
  **与 P3.1–P3.7 完全相同，本批新增 0**（这 81 行就是 P2.3 已归因、P3.7 已复算的那批 rustfmt 重排）。
- 独立复刻一（**原始**行多重集，不做任何归一化，`/tmp/nw-rule/p38-raw.py`）：搬迁前 `mod.rs` 10,000 行、
  搬迁后 `mod.rs` + `body/control.rs` 10,056 行；**只在基准侧 13 行 = 接线 12 + 签名 1**；
  **只在本批侧 69 行 = 接线 48 + 注释 12 + 空行 6 + 容器 2 + 签名 1**；
  **两个方向的代码行都是 0**。那 1 行「签名」差异就是 `const fn prefix` → `pub(crate) const fn prefix`。
- 独立复刻二（**按定义项**逐项比对，`/tmp/nw-rule/p38-verify.py`，口径同判据、实现独立）：
  两侧各 **195 项**（该复刻的扫描器不把 `mod x;` 与 `const fn` 当独立定义项，因此比 xtask 的 198 少 3 项：
  `InterruptionSource::reason`、`InterruptionSource::prefix` 与 `mod.rs` 里另一个批次的 `stable_name`——
  本批只有前两项；所以本批进入逐项比对的是 25 项而非 27 项。两个数字口径不同，不可互比），
  **键集合双向差集 0**；**归一化正文行差异分类 `{}`（代码行 0、新增注释行 0）**；
  **压缩串逐字节比对 195/195 相同，含本批 25 个搬迁项（25/25）**；落点核对：25 项全部落在 `body/control.rs`。
- 定义名双向差集（`--list-items`）：两侧都是 **198 项**，双向差集 **0**；本批 25 个目标项
  （`Store` 的 9 个方法 + 5 个结果类型 + 7 个常量 + 4 个 payload）在两侧**各出现且仅出现一次**
  （按名字计数时 `InterruptionSource` 会出现 6 次、`QuestionAnswerResult` 2 次等都是「定义 + 方法签名里的引用」，
  两侧对称）。
- 调用方未动：`git status --porcelain -uall` 共 **48 个路径**（`crates/` 内 **46 个**，比 P3.7 多出的两个就是
  `body/mod.rs` 与 `body/control.rs`），逐项归类后**声明外 0**；`crates/` 之外的唯一两个路径是
  `.cargo/config.toml`（+1 行空行，mtime 早于本批）与 `docs/08-giant-file-split-plan.md`（本批文档）。
  与 P3.7 快照逐字节比对：**新增 2 个文件（`body/mod.rs`、`body/control.rs`）、内容变化 1 个文件（`mod.rs`）**，
  其余全部（11 个兄弟模块、`store/src/tests/`、另外五个 crate 的 `src/`）**一字未改**。
- 测试身份：本批没搬测试；`cargo test -p nekowriter-store` **96 通过 / 0 失败**、`--list` 仍 96；
  `cargo test -p nekowriter-app -- --list` **69**（67 + 集成 2）；两个 crate 一起跑也是 **165 个用例、0 失败**。

**判据本身一行未改**（`xtask/src/main.rs` 仍 2,133 行；本批只把它列进 `--allow`）。

**门禁（2026-09-16，本批实测）**：`cargo fmt --all -- --check` 通过、`git diff --check` 通过；
逐包 `cargo clippy --all-targets` 六个 crate 全部 **0 warning / 0 error**；
`cargo clippy --workspace --all-targets` 强制重编译（只 touch 已存在的 6 个文件，六个 crate 全 `Checking`）
**0/0**；`cargo test --workspace` **269 通过 / 0 失败**（agent 44、app 67 + 集成 2、core 38、prompt-lab 2、
**store 96**、xtask 20）；`pnpm lint` **0 问题**、`pnpm test` **10 通过 / 0 失败**（2 个文件）。

**本批文件清单**：`crates/nekowriter-store/src/preparation/mod.rs`、
`crates/nekowriter-store/src/preparation/body/mod.rs`（新建）、
`crates/nekowriter-store/src/preparation/body/control.rs`（新建）、`docs/08-giant-file-split-plan.md`。
**下一批基准快照：`/tmp/nw-split-state/P3.8`（119 个 Rust 源文件）**，与工作区逐字节一致。

**P3.8 之后 `mod.rs` 的现状（下一批的对照数字）**：**8,617 行**（窄核口径：定义项 198 → 198、
项内行 8,823 → 8,823、残留 0）。`preparation/` 目录现有：`mod.rs` 8,617、`commands.rs` 775、`citations.rs` 667、
`body/`（`mod.rs` 7 + `control.rs` 1,439）、`tools/`（`mod.rs` 12 + `model.rs` 363 + `free.rs` 844 + `held.rs` 718），
目录合计 13,442 行。

**下一批是按项名执行**（P3.9 `body/edit.rs`：原 1598–2763 的 6 项——`get_chapter_body` 的文档行
现在在 **215**（`impl Store {` 在 214、`pub async fn get_chapter_body(` 在 216），即本批之后
`mod.rs` 里所有大于 1597 的旧行号整体前移 **1,383** 行）。

### 2026-09-16 P3.9：正文写入与提交 → `preparation/body/edit.rs`（未提交）

**项名核对：计划那一行的 6 个名字全中**（「计划列了、文件里没有」= 空）。

| 段 | 搬前行区间 | 项 | 行数 |
| --- | --- | --- | ---: |
| A | 215–1380（1,166 行 = 6 项 + 5 个项间空行） | `get_chapter_body`、`get_document_revision`、`append_chapter_body`、`replace_chapter_body`、`execute_chapter_body_agent_tool_call`（701 行）、`decide_chapter_body_proposal` | 1,161 |
| B | 124–126 | 3 个本层独占的 `*_METHOD` 常量 | 3 |
| C | 151–155、157–162、164–169 | 3 个本层独占的私有 payload DTO | 17 |

合计 **12 个定义项 / 1,181 行**（连 5 个项间空行、3 个 payload 尾随空行与方法区尾随空行，
共从 `mod.rs` 删掉 1,190 行）。

**一处刻意的留白**：行 121 的 `DECIDE_PREPARATION_REVIEW_METHOD` 仍在原地（P3.7 就留住的那一个，
P3.11 的三个 `decide_*` 与它共用；本批的 `decide_chapter_body_proposal` 用的是**另一个**常量
`DECIDE_CHAPTER_BODY_PROPOSAL_METHOD`，它在块外没有任何引用，因此随本层搬走）。三个 payload 同理：
`DecideChapterBodyProposalPayload` 的块外引用为空，P3.11 复用的是 `DecidePreparationReviewPayload`
（也在原地）。

**方法区仍然不出 `impl Store` 头**：方法区前一行 214 是 P3.7/P3.8 留下的同一个 `impl Store {`；
本批删掉 215–1381（含方法区后面那个空行）之后，它下面直接是 P3.10 的
`commit_chapter_body_model_submission`（`mod.rs` 里这一处的方法头计数始终是 1）。

**搬迁量**：`mod.rs` **8,617 → 7,423 行**；`body/mod.rs` **7 → 10 行**（+空行 +说明注释
+`pub(crate) mod edit;`）；`body/edit.rs` 新建 **1,238 行** = 5 行模块文档 + 39 行导入表（10 条 `use`）
+ 3 行常量 + 3 个 payload（17 行）+ `impl Store` 块（71–1238，1,168 行 = 1,166 行方法区 + 头尾两行）
+ 6 行分段空行。`mod.rs` 的 8,617 → 7,423 的账：−1,190（搬迁区间与尾随空行）
−4（`use super::model_requests::{…}` 因只剩两个名字从 3 行并成 1 行、`use nekowriter_core::{…}`
删掉 8 个失效名字后从 24 行排到 22 行）。

**接线：本批一个再导出都不需要**——六个都是 `Store` 的固有方法（调用方一律按方法语法调用，
这些名字从未出现在任何 `use` 里），常量与 payload 只在本层内部使用。`mod.rs` 这侧只删了因搬迁
失效的 `use` 名字（`AppendChapterContent`、`DocumentRevisionView`、`ReplaceChapterContent`、
`continue_chapter_body_after_tool`、`hold_chapter_body_proposal`、`hold_chapter_body_tool_for_pause`、
`stop_writing_after_protocol_error`、`next_protocol_error_count`，由编译器 `unused_imports` 点名）。
**可见性一处未改**（本批不读写任何被搬项之外的私有字段，`pub(crate)` 0 处）。

**导入表（P3.5 口令 + 本批的三处现场修正）**：① 机器扫描给出的 32 个候选名字里，第一次 `cargo check`
报了 13 处 `E0425`——**全是 `mod.rs` 自己的自由函数**（`chapter_body_snapshot`、`chapter_and_document`、
`chapter_body_run`、`current_chapter_body`、`persist_workflow`、`chapter_body_revision_dependency_ids`、
`insert_document_revision_with_payload_and_source`、`insert_revision_dependency`、
`insert_chapter_edit_review`、`update_chapter_body_run_status`、`workflow_for_version`、
`ensure_expected_state_revision`），按 P3.8 的 `super::super::…` 接回；② 与 P3.8 同一条：
`tools::model` / `tools::free` / `citations` 那三组名字也走 `super::super::…`，`mod.rs` 的再导出行
一个不删；③ **sea_orm 的方法式 trait 依旧不在名字扫描里**（`.transaction` 要 `TransactionTrait`、
`.insert` 要 `ActiveModelTrait`），而 `ColumnTrait` 本批用不到，`unused_imports` 点名后删。

**三层证据**

- 单批隔离（`--base-state /tmp/nw-split-state/P3.8`、`--from preparation/mod.rs`、
  `--into preparation/mod.rs + body/mod.rs + body/edit.rs`，**不给 `--allow`**）：退出码 **0，MOVE-ONLY OK**；
  定义项 **171 → 171**、项内行 **7,561 → 7,561**、项外自由行 **187 → 200**、**残留 0**、
  放行新增注释 **13 行**（= `edit.rs` 的 7 行（5 行模块文档 + 2 行导入说明）+ `body/mod.rs` 的 6 行——
  后者不在 `--from` 里，所以它整份文件的注释行都算新增；`mod.rs` 本批一行注释都没加）。
  **项内行两侧完全相同 ⇒ 本批一行 rustfmt 重排都没有**。
  *（口径提醒：`--into` **不要**把本批没动的 `body/control.rs` 列进去——那样它的 25 个 P3.8 项会全被
  算成本批新增，实测会把残留抬到 1,254 行。第一次跑就撞上了，与 P3.7 的「`--into` 必须随新增模块同步
  扩一个条目」正好是一对相反的错误。）*
- 整栈核对（`--base HEAD`，4 个 `--from` / **45 个 `--into`** = P3.8 的 44 个 + `body/edit.rs`）：
  定义项 **616 → 616**、项内行 **22,002 → 21,989**、项外自由行 699 → 970、**残留 81 行**——
  **与 P3.1–P3.8 完全相同，本批新增 0**。
- 独立复刻一（**原始**行多重集，不做任何归一化，`/tmp/nw-rule/p39-raw.py`）：搬迁前 `mod.rs` 8,617 行、
  搬迁后 `mod.rs` + `body/edit.rs` 8,661 行；**只在基准侧 17 行全是接线**（导入表的续行）；
  **只在本批侧 61 行 = 接线 48 + 注释 7 + 空行 4 + 容器 2**；**两个方向的代码行都是 0**。
- 独立复刻二（**按定义项**逐项比对，`/tmp/nw-rule/p39-verify.py`，口径同判据、实现独立）：
  两侧各 **195 项**（`BEFORE` = `mod.rs` + `body/control.rs`，`AFTER` = 两者 + `body/edit.rs`），
  **键集合双向差集 0**；**归一化正文行差异分类 `{}`（代码行 0、新增注释行 0）**；
  **压缩串逐字节比对 195/195 相同，含本批 12 个搬迁项（12/12）**；落点核对：12 项全部落在 `body/edit.rs`。
- 定义名双向差集（`--list-items`）：两侧都是 **171 项**，**项级双向差集 0**（只多出 11 行「项外」注释，
  全部是新模块的文档头）；本批 12 个目标项在两侧的按名字计数**逐一相等**。
- 调用方未动：`git status --porcelain -uall` 共 **49 个路径**（`crates/` 内 **47 个**，比 P3.8 多出的一个就是
  `body/edit.rs`），逐项归类后**声明外 0**；`crates/` 之外仍是 `.cargo/config.toml` 与 `docs/08…`。
  与 P3.8 快照逐字节比对：**新增 1 个文件（`body/edit.rs`）、内容变化 2 个文件（`mod.rs`、`body/mod.rs`）**，
  其余全部（11 个兄弟模块、`store/src/tests/`、另外五个 crate 的 `src/`）**一字未改**。
- 测试身份：本批没搬测试；`cargo test -p nekowriter-store` **96 通过 / 0 失败**、`--list` 仍 96；
  `cargo test -p nekowriter-app -- --list` **69**（67 + 集成 2）。

**判据本身一行未改**（`xtask/src/main.rs` 仍 2,133 行；本批只把它列进 `--allow`）。

**门禁（2026-09-16，本批实测）**：`cargo fmt --all -- --check` 通过、`git diff --check` 通过；
逐包 `cargo clippy --all-targets` 六个 crate 全部 **0 warning / 0 error**；
`cargo clippy --workspace --all-targets` 强制重编译（只 touch 已存在的 6 个文件，六个 crate 全 `Checking`）
**0/0**；`cargo test --workspace` **269 通过 / 0 失败**（agent 44、app 67 + 集成 2、core 38、prompt-lab 2、
**store 96**、xtask 20）；`pnpm lint` **0 问题**、`pnpm test` **10 通过 / 0 失败**（2 个文件）。

**本批文件清单**：`crates/nekowriter-store/src/preparation/mod.rs`、
`crates/nekowriter-store/src/preparation/body/mod.rs`、
`crates/nekowriter-store/src/preparation/body/edit.rs`（新建）、`docs/08-giant-file-split-plan.md`。
**下一批基准快照：`/tmp/nw-split-state/P3.9`（120 个 Rust 源文件）**，与工作区逐字节一致。

**P3.9 之后 `mod.rs` 的现状（下一批的对照数字）**：**7,423 行**（窄核口径：定义项 171 → 171、
项内行 7,561 → 7,561、残留 0）。`preparation/` 目录现有：`mod.rs` 7,423、`commands.rs` 775、
`citations.rs` 667、`body/`（`mod.rs` 10 + `control.rs` 1,439 + `edit.rs` 1,238）、
`tools/`（`mod.rs` 12 + `model.rs` 363 + `free.rs` 844 + `held.rs` 718），目录合计 13,489 行。

**下一批是按项名执行**（P3.10 `body/commit.rs`：原 2765–3150 的
`commit_chapter_body_model_submission` + 它的协议纠正常量与分支——`impl Store {` 现在在 **187**、
该函数在 **192**（文档行 **188**），即本批之后 `mod.rs` 里所有大于 1381 的旧行号整体前移 **1,194** 行）。

**下一批的两个待定项（P3.9 收工时的观察，留给 P3.10 按依赖闭包定）**：该区间里还有两个计划那一行
没点名的项——`mutate_chapter_body`（现行 **575** 行起，私有方法，本批的 `append_chapter_body` /
`replace_chapter_body` 通过 `self.mutate_chapter_body(…)` 调它，而语法调用不产生 `use`，所以搬迁不受
它牵制）与 `interrupt_active_*` 三个收束命令（现行 3,751/3,879 起，P3.8 已把它们的可见性依赖
`InterruptionSource::prefix` 一并处理）。
