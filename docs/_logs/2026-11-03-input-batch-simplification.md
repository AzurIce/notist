# 输入批次简化与完整负载验收

## 结果

输入模型改为按轮懒加载：同一读取能力下保留每个路径首次读到的字节、错误或 Pending，下一轮新建 Vault／ResourceSession 接收变化。不承诺整个目录的原子快照，也不再持续检查磁盘并使整批失效。

- 删除 ResourceRevision、ResourceReader.revision、ResourceSession.validate、Vault.validate_inputs、InputError.Changed，以及版本列表、消费结果比对、全局 changed 状态与 DocumentInputs 特例。没有兼容空壳、扫描开关或严格模式。
- ResourceReader 只需实现 read_bounded。文件系统首次读取仍检查目录 capability、普通文件、长度与同一文件句柄读取前后的元数据；读取中变化是该文件的普通 IO 失败，不毒化其他输入。
- 保留授权、单文件／总缓存／请求预算、依赖登记、诊断来源及 NeedResources 准备重试。缓存命中不访问后端，但仍检查授权并计费。
- 数据缓存按实际读取能力隔离，源码字节只有经同一个 rooted reader 加载，才能直接复用为物理数据。收窄授权不能借宽根缓存跨越 symlink 边界，source-only 后端不能借源码缓存获得数据能力。
- 每路径首次登记的显式文档文本作为本轮数据输入；已有读取保持原结果，后续登记不替换或使批次失效。SSG 仍在求值前登记全部生成／编辑页面。
- CLI 重建、LSP 与浏览器准备重试原本就创建新 Vault；comemo 编译／绑定缓存独立保留，没有重做调度或缓存整页输出。

不含已有 SSG 宿主额度修改的八个生产 Rust 文件净减 206 行，ResourceSession 从 455 行缩至 318 行。设计、Path、Data 与 panic 文档已同步。

## 完整 CLI 同批对照

基线为已提交的 `083ff6cd5eecae00a085a113a211f0f314ba0ae3`，另一侧为本次生产代码。两侧均在新的隔离目录编译普通 release 二进制，并应用完全相同的既有 SSG 宿主政策：requests=1,000,000、Data cache=512 MiB／65,536 entries。这项先前已有的政策不计入本次优化。

固定 CPU 8，每个版本／负载先完整预热一次，再在新进程中交错执行三轮；文件系统缓存已预热，CPU 频率未锁定。计时覆盖 CLI 启动、准备扩展、分析、求值、HTML、模板、事务发布及退出；Rust 编译、产物校验和指纹计算均在计时外。

以下均为本批普通二进制的三轮中位数，范围为最小至最大值；不混用旧报告秒数或插桩耗时。

| 完整负载 | HTML 页数 | 基线 s（范围） | 简化后 s（范围） | 中位数比值 |
| --- | ---: | ---: | ---: | ---: |
| 完整 XIV DB | 51,739 | 3.5916（3.5802–3.6520） | 3.4275（3.3496–3.7844） | 1.05× |
| 无模块正文 | 521 | 1.9109（1.8991–1.9321） | 0.4643（0.4636–0.4690） | 4.12× |
| 共享组件图 | 521 | 6.4423（6.4385–6.4623） | 0.7997（0.7990–0.8049） | 8.06× |
| 每页私有模块图 | 521 | 39.0120（39.0078–39.0267） | 1.0304（1.0270–1.0305） | 37.86× |
| 宽导出／通配绑定 | 521 | 5.3731（5.3638–5.3748） | 1.4707（1.4684–1.4713） | 3.65× |

完整 XIV DB 的范围重叠，只视为基本持平，不声称显著提速。该站点通过准备扩展生成大量页面，没有 notc 模块编译；它不能代替物理源码／模块图负载。

完整保留真实 XIV 输入、全部 51,739 页及正文；另四种负载均为 512 详情＋8 目录＋首页。未修改消费者、依赖锁、site.data 或页面枚举模型。

### 输出与实际工作验证

共执行 50 次完整构建：10 次预热、30 次普通计时、10 次独立阶段／模块探针。每次退出后都对全部相对路径和逐文件 SHA-256 与历史完整清单比较，50 次全部一致，合计校验 538,400 个文件。XIV 每次 51,744 个文件，其余每次 524 个文件。

独立探针的编译、绑定、链接、安装调用数在新旧版本间逐项相同；下表列实际缓存未命中的工作次数，不拿探针秒数替代普通计时：

| 负载 | 实际模块编译（两侧相同） | 实际导入绑定（两侧相同） |
| --- | ---: | ---: |
| 完整 XIV DB | 0 | 0 |
| 无模块正文 | 0 | 0 |
| 共享组件图 | 18 | 139 |
| 每页私有模块图 | 5,120 | 5,632 |
| 宽导出／通配绑定 | 6 | 11 |

私有图仍实际编译 5,120 个模块，未以少做模块工作、缩减页面或发布旧输出换取提速。本批不采集或排名 RSS，避免 Python fork 前高水位污染 Rust 峰值内存解释。

## 回归与检查

- 全 workspace、全部特性：725 项通过，零失败、零忽略；包含 Book／SSG 生成页、CLI 发布、serve、LSP、PreparedInputs、Data 来源及多 Analysis 环境隔离。
- 顶层 notist 无默认特性：365 项通过。CLI 无默认特性 cargo check 通过。
- 输入会话回归：19 项通过。新增覆盖首读固定、缺失／删除／后创建、懒加载、Pending／IO 错误缓存、晚登记不毒化、授权收窄、symlink、source-only 后端、后台读取次数及文档首次登记。
- 新增 Book／SSG 实时重建回归，验证修改、删除、恢复 JSON 输入均在新一轮生效，且私有输入的 HTTP 路径始终为 404。
- Python 工具：22 项通过；浏览器准备与组件协议：24 项通过。历史资源探针单测锚定旧提交，不要求当前引擎继续保留旧校验 API。
- Clippy 全 workspace／all-features／all-targets 通过；三个已有警告位于未改的 `crates/notist-pipeline/src/context.rs`、`crates/notist-pipeline/src/profiling.rs`、`crates/notist-cli/src/serve.rs`。本次变更文件 rustfmt 与 git diff --check 通过。
- 全仓 cargo fmt --check 仍报告三个未改的基线文件：`crates/notist-pipeline/src/frontend/markdown/mod.rs`、`crates/notist-pipeline/src/frontend/markup/mod.rs` 的排版，以及 `src/lib.rs` 的导出排版／顺序；没有为此改动无关生产代码。
- 旧 Option serde 测试对含 NaN 值使用普通相等断言，已在旧基线独立复现失败；修正为检查完整结构与浮点位级 key，没有改变语言的 NaN 相等语义。
- workspace 测试使用 NO_COLOR=1、TERM=dumb 和单测试线程，避免既有终端颜色断言及本地 serve 并发连接竞态；完整套件没有跳过测试。环境未安装 wasm32 目标标准库，未声称完成 wasm32 编译。

## 产物与复现

本批原始目录：`/tmp/opencode/notist-input-batch-20261103/`。

- `verify_input_batches.py`：准备、隔离编译、计时、独立探针与完整逐文件校验脚本。复现时将脚本复制到新的结果目录，在基线 HEAD 加本次未提交补丁的检出上依次运行 prepare、build、measure、probes；旧目录保留作证据。
- `manifest.json`：工具链、系统、宿主额度、两侧普通／探针二进制哈希、源码与输入指纹。
- `sources/`、`probe-sources/`、`bin/`：完整源码快照、独立插桩及四个 release 二进制。
- `samples.json`、`summary.json`、`probes.json`、`logs/`、`inventories/`：全部样本、工作计数、完整 stdout／stderr、逐次路径与字节指纹。
- `integrity.json`、`checkout-final.diff`：独立复核 50 次产物、源码／二进制指纹与最终生产代码一致性，以及最终已跟踪文件补丁。
- `workspace-tests-accepted.log`、`no-default-tests.log`、`host-input-batch-test.log`、`clippy-final.log`、`python-tests.log`、`browser-protocol-tests.log`：验证记录。

上述验收基于提交前的工作区完成；工作区原有 SSG 政策修改、历史性能工具、报告与截图均予以保留。
