# 修复报告：docs/05 全仓工程复核的处置

处置对象是 `docs/05-full-repository-engineering-review.md` 的 F-01…F-18（另含 P4 仅记录项）。
本报告只写**已落地的事实**：每一条都给出改在哪、用什么钉住、现场在哪一节。
逐条现场证据（命令、请求、数据库片段）在 `docs/03` 的 §45；本报告不重复粘贴。

## 1. 摘要

| 类别 | 数量 | 说明 |
| --- | --- | --- |
| 已修复 | 15 | F-01…F-08、F-10…F-12、F-14…F-18 |
| 按决定收窄 | 1 | F-13：目录与纯逻辑用例建立起来了，hook / IndexedDB 层有意不覆盖（见 §9） |
| 有意延期 | 1 | F-09：万行级纯搬迁拆分，理由见 `docs/02` §2.4 |
| 新发现并当场修掉 | 1 | 静态回落把**缺失的静态资源**变成 200 + HTML（复核报告未列出，验证 F-01 时撞到） |

三类根因：

1. **抽象只迁移了一半**——幂等收尾（当时的 `finish_write`）与 `insert_app_event`（事件写入）都已存在且语义等价，
   但老模块的调用点没迁。修法是把剩下 32 个调用点迁到同一处，而不是再造一层抽象；
   收口后又把这套东西换成了持有 `command_id`/方法/目标/指纹的幂等信封（`Command`）与
   `Notify` 类型（取代 49 个 `notify: bool` 实参：46 个字面 `true` + 3 个按事务结果计算的值），调用点因此不再复述它——见 F-05 行与 §45.2。
2. **同一份规则写了两遍**——持久事件名两侧各写一份、API 根地址校验一严一松、
   界面能力在一次外壳重写里被写到新外壳之外（代码还在，只是没人挂载）。
   修法是让「两侧一致」变成可执行断言、让规则只留一份、让被丢的能力重新可达。
3. **门禁与文档落后于实现**——前端 lint 静默失效、没有 CI、`docs/02` §2.1 的模块表写着不存在的模块、
   §16.1 停留在已关闭的旧状态。修法是让门禁真的会失败、让文档写当前事实。

## 2. 原始发现闭环矩阵

| 发现 | 状态 | 处置 | 测试 / 证据 | 备注 |
| --- | --- | --- | --- | --- |
| F-01 两个已承诺的界面能力不可达（快捷键说明、本地服务面板） | 已修复 | `ShortcutHelpPanel` 挂进 `App.tsx`，`ServicePanel` 挂进 `pages/SettingsPage.tsx`；两者依赖的事件同步状态移到 `lib/event-sync-context.ts` 并补 `/api/health` 探针与 `serviceStopped` | Rust 用例 `restored_ui_capabilities_are_mounted_in_the_app_shell`（源码级挂载对账）+ 真界面点击（§45.4.1、§45.4.2） | 恢复必须连带健康检查状态，否则退出按钮是死按钮 |
| F-02 前端 lint 门禁静默失效（8 → 30 error） | 已修复 | `use-workspace-draft.ts` 改渲染期只读（见 F-14）、`use-mobile` 改 `useSyncExternalStore`、`UserQuestionPanel` 改 `key` 重挂载、4 个阶段面板的 `persistedXxxRef` 改为渲染可见 state + 单写入 latest-ref、`ui/*` 的 react-refresh 靠拆分变体解决 | `pnpm lint` → **0 error / 2 warning**（§45.3）；两条虚拟化库 warning 已在 §46.4 按文件收窄，现为 **0 error / 0 warning** | 两条虚拟化库 warning 已在 §46.4 按文件收窄为 0（§9 第 4 条，已关闭）；「8 → 30 error」是复核时点（`HEAD a8374ff`）的基线差 |
| F-03 `.cargo/config.toml` 用 `force = true` 钉死 Windows 路径的 `PROTOC` | 已修复 | `PROTOC` 只在 `[target.x86_64-pc-windows-msvc]` 下设置；非 Windows 让构建脚本在 PATH 上找，安装方式写进 `tools/README.md` | 本地非 Windows 构建不需要仓库外配置即可开始（§45.6） | 与 `docs/02` §11 的措辞同步改成事实 |
| F-04 持久事件名在 Rust 与 TS 各写一份，且 4 个类型前端从不订阅 | 已修复 | `store/event_types.rs` 成为唯一清单（`EVENT_TYPES`），TS 侧 `persistentEventTypes` 与它逐项相同 | `manifest_is_sorted_and_unique`、`every_written_event_type_is_in_the_manifest`（扫生产代码里所有写入点）、`frontend_subscribes_to_exactly_the_manifest`（直接读 `events.tsx`） | 「两侧一致」现在是可执行断言，不是纪律 |
| F-05 幂等收尾只迁移了一半（`finish_write` 17/49） | 已修复 | 第一轮把剩下 32 个站点迁到同一收尾路径；收口轮把它换成**幂等信封**：`Command { id, method, target, payload_hash }` + `Notify { Listeners, Quiet }`，46 个站点用 `command.commit(store, notify, body)` 一行完成「预查 → 事务体 → 收尾」，3 个「按结果决定通知」的站点用 `command.finish(store, notify, result)`，3 个只做预查的 helper 用 `replay_if_recorded`；`finish_write` 自由函数已删除。预查去重同轮收口：删掉 41 处「站点预查 + `commit` 又预查一遍」的纯重复，`commit` 成为事务外预查的唯一所有者 | `cargo test --workspace`（F-05 收口那一轮 store 94；当前树 store 96、全仓 253，见 `docs/03` §46.5）+ 逐模块计数与 `wc -l` 对照（§45.2）+ `command_finish_replays_an_existing_receipt_for_a_non_database_error`（钉住失败分支语义）+ `a_no_op_command_writes_its_receipt_but_does_not_wake_subscribers`（钉住「no-op 不通知」）+ `transaction_external_recheck_has_a_single_owner`（钉住预查唯一所有者）+ 与 `HEAD` 二进制的 SQL trace 对照（2 → 3 → 2，§45.2） | 每个命令站点仍自己写事务体（算指纹 → 业务写入 + `insert_receipt` → 提交）：回执必须与业务写入同事务，有意不搬；并发重复由 `command_receipts.command_id` 主键冲突兜底。（`docs/05` 的 S2 与 P3 里写的「事务内复查」是评审当时对代码的误读：当时与现在都没有这一步，事实口径以 `docs/02` §2.4 与 `docs/03` §45.2 为准。）白名单内的 5 个知识库站点保留早退预查（省 `embedding_profile_with_id()` 读库，`reindex` 还省整表扫描），3 个 `finish` 站点保留预查（`finish` 不预查）。**一处有意的非等价改动**：收尾的失败分支从「只认 `StoreError::Database`」放宽为「任何错误都先查回执」，理由与代价见 §3.1。行为等价性另由四次反向实验证明：通知判定两次（§45.2 实验 A/B）、预查唯一所有者两次（实验 C/D） |
| F-06 事件写入被 3 个模块绕过 | 已修复 | `settings.rs` / `project_settings.rs` / `prompts.rs` 改调 `insert_app_event`，删掉手搓 `event::ActiveModel` | 生产代码里 `rg "event::ActiveModel"` 只剩正典 `insert_event_row` 内部一处 | 与 F-04 同时做，事件清单才有唯一来源 |
| F-07 两条设置命令平行实现（校验严宽不同） | 已修复 | 统一到一份严格实现 `validate_api_base_url`（`store/lib.rs`），模型侧与 Embedding 侧都用它；app 侧编排也收拢 | `api_base_url_rule_is_one_rule_for_both_settings`、`embedding_requests_accept_the_payload_the_ui_sends` | 「必须是 API 根路径」不再有两套判定 |
| F-08 `docs/02` §2.1 模块表写着 6 个不存在的模块 | 已修复 | §2.1 改成当前真实模块路径与职责，并说明最后几行是「今天把多关注点放在同一文件里」的边界；§2.4 列出长期拆分项 | 文档与 `crates/*/src` 对账 | 维护者按文档找位置不再扑空 |
| F-09 万行文件与 678 行的错误映射函数 | 有意延期 | 不做：纯搬迁的万行级改动与行为修复混在同一批文件里会让「哪一处改动改变了行为」无法核对；`docs/02` §2.4 已写明方式（只移动、不改行为，单独一轮） | — | §9 记录为未关闭项 |
| F-10 写信任逐个 handler 手写 + 文档承诺的自定义头不存在 | 已修复 | 改成路由层中间件 `require_trusted_write`，删掉 **53** 处手工调用（另删 2 处 `use` 行；`ensure_trusted_write_request` 现在只剩中间件那一次调用与定义）；`docs/02` §11.3 改成事实（JSON + `Host`/`Origin`，不再声称自定义头） | 路由级用例 `write_trust_is_enforced_by_the_router_not_by_handlers`（`Origin` 403 + `Host` 400）+ 同一个 handler 不做校验也能被拦下 | 新增写端点不再可能漏掉这条校验 |
| F-11 没有 CI | 已修复 | `.github/workflows/ci.yml`：`rust`（protoc、toolchain、fmt、clippy、test）与 `webui`（install、lint、test、build） | 本地逐条等价命令全绿（§45.6）；YAML 结构用 `ruby -ryaml` 解析确认（§45.3） | 未在 GitHub 上实跑（§9） |
| F-12 请求快照与命令回执无界增长、无口径 | 已修复（按复核建议的方向） | **不做**自动清理（会破坏审计链）；`store/growth.rs` 在启动时把各表行数与重文本字节数写进日志，口径写进 `docs/02` §11.3 | 启动日志实际输出 `本地数据规模 table=model_requests=0行/0字节 ...`（§45.4） | 「谁负责长期回收」现在有明确答案：暂不回收，但可诊断 |
| F-13 前端零自动化测试、文档承诺的 Playwright 层不存在 | 按决定收窄 | 引入 vitest（`pnpm test`），只跑纯逻辑：HTTP 失败映射/重试/命令 ID、输入法组合判定；`docs/02` §15.1 改成事实（人工真浏览器验收，没有 Playwright） | 10 个用例通过（§45.3） | hook 与 IndexedDB 层有意不覆盖，见 §9 |
| F-14 `use-workspace-draft.ts` 在 render 体内读写 ref | 已修复 | 渲染期只读 state：最近编辑内容由 effect 写入单份镜像，identity 变化时的补写读那份镜像；`use-mobile` 改 `useSyncExternalStore` | `pnpm lint` 的 `react-hooks/refs` 归零（§45.3） | 未构造出反例（当时无测试基建），处置是「按规则重写」而不是「证明修好了一个 bug」 |
| F-15 `command_receipts.payload_hash` 覆盖密钥字段 | 已修复 | 密钥不进摘要（`CredentialPatch` 三态参与合并但不参与 `payload_hash`）；`settings.rs` 那句自相矛盾的注释同步改掉 | 同 `commandId` 只换密钥 → 200 重放、`revision` 不增、回执与事件都不增（§45.5） | 回执仍能挡住同 ID 异体：`expected_revision` 与其余字段在摘要里 |
| F-16 没有任何路径把 API key 清回「未配置」 | 已修复 | `CredentialPatch::Clear`（HTTP `apiKey: null`）+ store 写事务置 NULL + 界面「清除密钥」两步确认 + `docs/02` §8.4 删除规则表 | store/app/HTTP 三层用例 + 真界面两步确认与数据库对账（§45.4.3） | 「未配置」现在既能读出也能写回 |
| F-17 死代码（两个未挂载组件、三个无人引用的导出/getter） | 已修复 | 两个组件随 F-01 恢复挂载（因此不再是死代码）；`commandKeyLabel` 与 `provider::api_base_url()` 删除；`useWorkspaceFocusElement` 已被工作台使用，保留 | 全仓引用扫描 | 死导出不再靠人工发现（lint + 事件清单对账已是门禁） |
| F-18 `docs/02` §16.1 的过期陈述 + `.agents/skills/**` 入库 | 已修复 | §16.1 那句已不存在，其余条目逐条改到当前事实（含本轮新截到的退出后界面状态）；`.agents/` 与 `skills-lock.json` 移出版本控制并加进 `.gitignore`；`NekoWriter.txt` 移到 `docs/00-original-requirements.txt`（`docs/01` 与 `docs/02` 的链接同步改到新路径） | `git ls-files .agents` 为空；文件仍在磁盘上 | 索引层的改动需要维护者提交时确认（§9） |
| **N-01**（修复过程中新发现，不在复核报告里）：静态回落把**缺失的静态资源**变成 200 + HTML | 已修复 | 静态目录的回落只在「末段不带 `.` 的路径」上生效，带扩展名的路径如实 404；`/api` 保留自己的嵌套回落（JSON 错误信封）。引入与收口同在本轮，见 §3.1 | `static_dir_serves_client_routes_and_reports_missing_assets_honestly`（真实回环端口、真实路由）；修复前/修复后对照见 `docs/03` §45.1 | 前端是 hash 路由，刷新深链接**不**需要这个回落（服务端只看到 `/`）；它的实际价值是裸路径（在地址栏敲 `/settings`）不再得到空 404。带回落的版本若不带「带扩展名即 404」这条规则，正是 N-01 的来源 |

## 3. 根因修复（结构性改动）

### 3.1 两处需要点名说明的改动

**（a）幂等收尾的失败分支被有意放宽，不是行为等价。** 迁移前它是
`Err(StoreError::Database(error)) => 按回执重放`，其余错误原样返回；现在是
`Err(error) => 先查回执，有就还原成那条回执，没有就原样返回原错误`。
理由：事务外的预查挡住了绝大多数重放，但两次预查之间可能已经有同 `command_id` 的命令落库，
这时输的那一次会先撞上业务守卫（例如修订已被赢的那次推进），拿到的**不是**数据库错误，
而唯一正确的结果仍是那条既有回执。
安全性由 `replay_after_receipt_conflict` 保证：查不到回执就原样返回传入的错误；
查到但方法/目标/指纹不一致仍是 `IdempotencyConflict`（不会被放宽吞掉）。
代价与已知限制：这一支今天**只能由构造或并发真实抵达**，没有确定性触发路径，
因此由 `command_finish_replays_an_existing_receipt_for_a_non_database_error` 直接钉住信封收尾的契约：
它用一条真实命令留下真实回执，再以非数据库错误调用收尾；把分支收窄回
`Err(StoreError::Database(error))` 后该用例会以 `SystemSettingsRevisionConflict` 失败（本轮实测过）。

**（b）静态目录的回落范围被收窄（N-01）。** 本轮真正撞到的新问题是：带回落的服务把**所有**未命中路径
都变成 200 + HTML，包括缺失的静态资源（浏览器会把 HTML 当脚本执行）。现在的规则是末段带 `.` 即如实 404。
注意它的定位：带回落的版本不是任何复核条目的要求，而「带扩展名即 404」这条规则是 N-01 的修复；
两条规则一起才有意义，因此把回落本身与这条规则记在同一条目下。

## 3. 根因修复（结构性改动）

1. **一条规则一处实现**：幂等收尾（`Command::commit` / `Command::finish`）、事件写入（`insert_app_event`）、
   持久事件清单（`event_types.rs`）、API 根地址校验（`validate_api_base_url`）、
   写请求信任（`require_trusted_write` 中间件）各自只剩一份。
2. **横切规则用构造保证**：写信任从「57 处手工调用」变成路由层中间件，
   新增写端点不需要记得做任何事；事件类型清单从「两侧各写一份」变成对账断言。
3. **能力可达性有回归护栏**：`restored_ui_capabilities_are_mounted_in_the_app_shell`
   直接读前端源码断言四个挂载点存在——F-01 的失效模式（代码在、没人挂载）不会再静默发生。
4. **文档写当前事实**：`docs/02` 的 §2.1（模块表）、§2.4（剩余债务）、§11.3（写安全 + 静态/`/api` 边界）、
   §15.1（测试层次）、§16.1（已覆盖边界）都在本轮按实现校正；§8.4（清除密钥）、§9.3（地址校验归属）已在上一轮校正。

## 4. 删掉的冗余 / 过时路径

- 53 处 `ensure_trusted_write_request(&headers, &state)?` 手工调用（另 2 处 `use` 行；连同各 handler 的 `HeaderMap` 参数）。
- 3 处手搓事件行（`settings.rs` / `project_settings.rs` / `prompts.rs`）。
- `shortcuts.tsx::commandKeyLabel`、`provider.rs::ChatCompletionsConfig::api_base_url()`。
- 3 处事件写入的类型清单副本（TS 侧 `persistentEventTypes` 改为与 Rust 清单对账，而不是各自维护）。
- 版本控制里的 25 个第三方 agent 技能文件、`skills-lock.json`、根目录 `NekoWriter.txt`（改到 `docs/00-original-requirements.txt`）。

## 5. 现在被显式约束的不变量

| 不变量 | 在哪保证 | 用什么钉住 |
| --- | --- | --- |
| 同 `commandId` 的重放返回既有回执，不重复写、不重复发事件 | `store::Command::commit` | store 用例 + §45.5 现场 |
| 密钥正文只存在于本地 SQLite 的单列，只有一个读出口；不进响应、事件、日志、请求快照与回执摘要 | `CredentialPatch` + `system_model_api_key` / `embedding_api_key` | store/app 用例 + §45.4.3 的日志扫描 |
| 写请求只接受回环 `Host` 与允许来源的 `Origin` | 路由层中间件 | 路由级用例（`Origin` / `Host` 两路） |
| 持久事件类型两侧完全一致，且写入点都在清单里 | `event_types.rs` + 三个对账用例 | 扫生产代码的写入点 |
| 未知 `/api/*` 是 JSON 错误信封；静态回落只服务前端路由形状的路径，缺失资源如实 404 | `crates/nekowriter-app/src/main.rs` 的路由与 `serve_client_route` | `static_dir_serves_client_routes_and_reports_missing_assets_honestly` |
| 界面能力必须挂在应用外壳里 | 前端源码挂载点 | `restored_ui_capabilities_are_mounted_in_the_app_shell` |
| 本地数据体积可诊断 | 启动时 `storage_growth()` | 启动日志 + `docs/02` §11.3 的口径 |

## 6. 测试增补（与修复行为对应）

| 测试 | 钉住的行为 |
| --- | --- |
| `static_dir_serves_client_routes_and_reports_missing_assets_honestly` | 静态/`/api` 边界与回落规则（含缺失资源 404） |
| `write_trust_is_enforced_by_the_router_not_by_handlers` | 路由层写信任（`Origin` 403、`Host` 400、读方法不受影响） |
| `restored_ui_capabilities_are_mounted_in_the_app_shell` | 四个挂载点（两个组件 + 两处提示条） |
| `system_model_credential_can_be_cleared_over_http` | 密钥三态在 HTTP 上成立，且响应体从不带回密钥 |
| `manifest_is_sorted_and_unique` / `every_written_event_type_is_in_the_manifest` / `frontend_subscribes_to_exactly_the_manifest` | 事件清单与两侧一致 |
| `api_base_url_rule_is_one_rule_for_both_settings` | 两条设置命令共用同一份地址校验 |
| `command_finish_replays_an_existing_receipt_for_a_non_database_error` | 信封收尾的失败分支覆盖非数据库错误（放宽后的分支），且没有回执/指纹不符时仍返回原错误与 `IdempotencyConflict` |
| `a_no_op_command_writes_its_receipt_but_does_not_wake_subscribers` | no-op 命令（写入与既有值逐字相同）照样落回执，但不唤醒事件订阅者；反向实验见 §45.2 |
| `request_fingerprint_ignores_the_credential_body` | 回执指纹不含密钥正文，但保留「保留 / 替换 / 清除」三种意图与请求修订的区分 |
| `webui/src/lib/api.test.ts`（7 个） | HTTP 失败映射、写重试与命令 ID 稳定性、省略密钥的请求体形状 |
| `webui/src/lib/shortcuts.test.ts`（3 个） | 输入法组合判定（`isComposing` / `keyCode 229` / 普通 Enter） |

## 7. 迁移与兼容

- **数据库 schema 未变**（`SCHEMA_VERSION` 未动）：本轮没有列改名，清除密钥用的是既有 `api_key` 列的 `NULL`，
  因此已有数据目录照常打开。
- **HTTP 合同**：新增的只有 `apiKey: null` 的语义（省略 = 保留，`null` = 清回未配置），
  这是**新增**的合法取值，原有请求形状不变。
- **持久化 JSON / 事件载荷**：未改字段与类型名；事件清单只是把已有的名字集中到一处。
- **索引与打包**：`.agents/`、`skills-lock.json` 移出版本控制（文件仍在磁盘上）；
  `NekoWriter.txt` 移到 `docs/00-original-requirements.txt`。

## 8. 验证结果

```
cargo fmt --all -- --check                     干净
cargo clippy -p nekowriter-core -p nekowriter-agent -p nekowriter-store -p nekowriter-app --all-targets   0 error / 0 warning（本轮清零，见 docs/03 §46；包名必须写全，`-p core` 这类短名会以 "did not match any packages" 直接失败）
cargo clippy -p xtask --all-targets            0 error / 0 warning
cargo clippy --workspace --all-targets         0 error / 0 warning（本轮未开始时整个 workspace 还跑不了；测量过程见 docs/03 §46.6.3）
cargo test --workspace                         253 通过 / 0 失败（本仓 251：agent 44、app 单元 67、app 集成 2、core 38、store 96、xtask 4；另一线程的 nekowriter-prompt-lab 2）
webui pnpm lint                                0 error / 0 warning
webui pnpm test                                10 通过 / 0 失败
webui npx tsc --noEmit && pnpm build           干净
```

真实现场（`docs/03` §45）：静态/`/api` 边界的修复前与修复后对照；幂等收尾调用点计数与收口轮的预查去重；
写命令的回执查询次数与 `HEAD` 二进制逐条对照（2 → 去重前 3 → 现在 2）；密钥不进摘要的同 ID 换密钥现场；
以及全新数据目录上的四次真界面点击
（快捷键说明、本地服务面板 + 二次确认、清除密钥两步 + 数据库对账、SIGTERM 之后的「本地服务已退出」横幅）。
`docs/05` 里被复核报告点名「零断言」的路由级行为，现在由真实回环端口上的 HTTP 用例覆盖。

## 9. 仍未关闭的问题（真实存在）

1. **F-09（有意延期）**：`store::preparation`（万行级）、`app::scheduler`、`store::lib`、`app::main`、
   `core::lib` 仍是多关注点单文件。方式是只移动不改行为、单独一轮；混在本轮会让人无法核对行为变化。
2. **F-13 的收窄部分**：草稿 hook（含 IndexedDB 写入语义）没有自动化覆盖，需要 jsdom + `fake-indexeddb`；
   本轮只把纯逻辑层立了起来。「切换章节时不把上一章的文字写到下一章」因此仍依赖实现约束与人工路径。
3. ~~**clippy warning 仍与复核基线同量级**~~：**已关闭**（`docs/03` §46）。230 个唯一站点里 75 个机械项逐条改掉，
   剩下 155 个写成抑制：其中 **4 类**（`too_many_lines` / `too_many_arguments` / `missing_errors_doc` / `missing_panics_doc`）
   在 `Cargo.toml` 的 `[workspace.lints.clippy]` 里带理由关掉；另外 **2 类**框架约定项（`unused_async_trait_impl` /
   `struct_field_names`）不在批准的四类内，是按证据收窄到 `crates/nekowriter-store/src/entity/mod.rs` 的模块级允许
   ——这是一次 4 → 6 的口径扩张，已在 `docs/03` §46.3 更正并两侧验证（§46.6.2），现在 `clippy` 为 0 警告。
4. ~~**2 条前端 lint warning**~~：**已关闭**（`docs/03` §46.4）。该规则针对 React Compiler，而本仓没有接入编译器，
   覆盖按 `files` 收在 `ChapterDirectory.tsx` 与 `ManuscriptReader.tsx` 两个文件上（不是全仓关闭），并注明了恢复条件。
5. **CI 未在 GitHub 上实跑**：workflow 只做过 YAML 解析与本地等价命令执行；`protobuf-compiler` 的
   安装步骤与 Linux runner 上的完整链路没有真实运行证据。
6. **`.agents/` 移出版本控制只到索引层**：文件仍在磁盘、`.gitignore` 已加，但需要维护者在提交时确认这次索引改动。
7. **`docs/05` 的 P4 仅记录项**：前端注释中英混用、`lib/utils.ts` 只有 1 行、根目录可能残留 `app.log`——
   报告本身标注「仅记录，不要求修」，本轮未动。

## 10. 最终工程评估

- **可交付性**：`cargo fmt/clippy/test` 与前端 `lint/test/build` 全绿；真实端点的写作主链、真界面点击路径
  与退出/重启语义都有现场证据，不是只有单测。
- **结构一致性**：幂等收尾、事件写入、事件清单、地址校验、写信任各自只有一份实现；
  F-01 那类「代码在、没人接」的失效模式有了构造层的护栏。
- **可扩展性**：新增写端点自动获得写信任校验；新增持久事件类型必须进清单，否则对账用例会失败。
- **文档**：`docs/02` 的模块表、债务、写安全、测试层次与已知边界都与当前实现一致；
  剩余债务写成了「怎么做、为什么现在不做」，而不是含糊的待办。
- **结论**：复核报告里影响正确性与可维护性的条目已处置完；剩下的两项（巨型文件拆分、草稿层自动化测试）
  是明确记录、有路线、且不阻碍当前使用的技术债，不是未知风险。
