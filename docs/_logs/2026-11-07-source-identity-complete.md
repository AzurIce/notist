# 32 位 SourceId：完整所有权重构

## 范围与提交状态

- 按用户要求以 `a168131` 回退永久池原型 `40328bd`。
- 本次重新实现不复用原型的性能结论；也没有推进 DefId、BindingId 或 Symbol。
- 新实现、API 迁移和测试保持未提交；两份原有未提交分析草稿及 consumer 工作区未改动。

## 身份、修订与生命周期

`SourceId` 为私有 `NonZeroU32` 包装，ID 和 `Option<SourceId>` 都是 4 字节。
编号单调分配、不复用、检查耗尽。它不代表正文版本、资源授权或缓存有效性。

全局唯一化表只持有弱身份引用。最后一个身份拥有者释放时，清理对应弱表项及
来源键；并发新登记替换过期弱项时，旧析构不会删除新身份。表内不保存正文。
稳定 namespace 区分来源域；同名匿名来源分配独立身份。

`Source` 拥有身份和准确消费的不可变修订；`Sources` 是 COW 修订快照。
同一活跃来源的编辑保留相同 ID，但旧产物继续拥有旧正文。不兼容修订合并先做
完整检查，失败不部分修改目标；允许相同正文合并和 metadata-only 记录补充正文。

## 影响面的迁移

- 节点、checked IR、调用和展开链只传 ID。模块、checked program、analysis、
  content analysis、evaluation 和 HTML 映射结果显式持有必要来源。
- `Analysis::into_parts()` 返回树、诊断、Sources。`Located<T>` 用于独立拥有的树。
- 独立诊断拥有 Source；Data/DataError 的 read 位置使用拥有修订的 SourceSpan。
- 独立求值验证调用者提供的来源快照；扩展输出出现外部 ID 时拒绝并回滚。
- 共享特化缓存只保存模块源码，不夹带第一个调用文档的修订。缓存命中时再合并
  当前文档；已有模块/输入/前端缓存键和授权流程不变。
- Vault handle 校验来源身份和原输入 Arc；诊断定位使用自身修订，而非按名字
  找到另一个版本或偷偷借用主文档正文。
- CLI、LSP、JSON、preview、HTML、SSG、Book、索引和链接诊断迁移到显式来源解析。
  源码映射拒绝不完整来源并自行保活快照。纯 HTML 不必解析未使用的来源 ID。
- 裸 sourceful IR 不再实现 serde；借用 `sources.wire(...)` 仍输出原有来源名字。
  decode 只接受当前快照内无歧义的名字；import 显式指定 namespace，旧 wire
  不带正文时建立 metadata-only 记录，不全局反查或伪造正文，也不用 TLS resolver。
- core/eval/pipeline 仍可无 serde 使用；顶层 notist 的常驻 JSON API 显式启用
  core serde，不再产生原有无默认特性下的 TypedExpr serde 编译错误。

API 使用说明见 `docs/source-ownership.md`。

## 语义验证

- 全工作区全特性：791 项 Rust 测试通过，无失败/忽略。
- 全工作区无默认特性：775 项 Rust 测试通过，无失败/忽略。
- 单独 core/eval/pipeline 无默认特性：156 项通过。
- evaluator driver：58 个真实 fixture 场景语义验证通过。
- Python 工具测试：57 项通过；同步调整旧分阶段 profiler 的 HTML 锚点。
- 格式检查和普通 Clippy 通过；Clippy 仍报告原有 shared collection、调用参数数目、
  旧条件式等警告，不宣称严格 `-D warnings` 已清零。

新增回归覆盖超过 65,535 个来源、最后拥有者释放后的键清理、编号不复用、并发
唯一化、跨域同名、匿名身份、原子修订冲突、旧正文诊断、contextual wire 缺失/
歧义拒绝与 metadata-only import、独立求值、源码映射保活、扩展回滚及跨文档缓存。

## 完整站点测量方法

复现脚本为 `tools/profile_source_identity.py`。基线由 `git archive HEAD` 冻结为
`a168131`，候选冻结包含全部未提交 Rust 新文件。两侧使用相同 release 设置：
`CARGO_INCREMENTAL=0`、`CARGO_BUILD_JOBS=2`、release debuginfo=1；逐侧清理并重建
workspace 包，运行独立 native CLI。没有修改 consumer、依赖锁、默认预算或输入。

使用此前冻结的完整 icexiv 输入，53,102 个文件、711,605,507 字节；其中 51,225
个 item 页、513 个 catalog 页及首页。不是抽样 workload。初次发布不计入 clean
cohort；随后五对交错 before/after 完整构建，进程内缓存每轮重建，OS 文件缓存和
既有发布目录允许保留。clean 指无 profiler/分配插桩，并非冷磁盘首次构建。

wall/CPU/RSS 与分配诊断分开运行，不混用插桩耗时。每轮计时结束后逐文件校验
全部 78,004 项产物（451,812,631 字节）；cohort 前后另校验完整输入/源码哈希，
每轮启动前校验二进制哈希。分配计数为
API 请求流量，不冒充活跃堆大小或拷贝字节；RSS 是完整子进程峰值，不只求值器。

复现顺序（`--out` 必须是新的外部目录）：

```sh
python3 tools/profile_source_identity.py prepare --out /tmp/opencode/source-identity-REPRO
python3 tools/profile_source_identity.py build --out /tmp/opencode/source-identity-REPRO
python3 tools/profile_source_identity.py measure --out /tmp/opencode/source-identity-REPRO
python3 tools/profile_source_identity.py allocations --out /tmp/opencode/source-identity-REPRO
python3 tools/profile_source_identity.py phases --out /tmp/opencode/source-identity-REPRO
python3 tools/profile_source_identity.py verify --out /tmp/opencode/source-identity-REPRO
```

`prepare` 默认从 `/tmp/opencode/notist-icexiv-kernel-20261107-final` 读取冻结输入。
完整新证据位于 `/tmp/opencode/notist-source-identity-accepted-20261107`。该目录中的
`sources/before`、`sources/after`、manifest、独立二进制、原始样本、每轮 stderr 和
产物清单共同保留复现依据。`validation/summary.json` 校验当前工作区全部 246 个
Rust/Cargo 文件与测量冻结源一致，并保存测试日志与 evaluator 场景校验。

最终补齐 LSP metadata-only 诊断的缺正文提示后重新冻结、重建并整组重跑；此前
样本保存在 `superseded-before-metadata-diagnostic/`，不并入最终统计、不择优混用。

## 最终测量结果

下表均为最终候选的同组中位数，不是被回退原型的数据。

| 无插桩完整构建 | before | after | 变化 |
| --- | ---: | ---: | ---: |
| wall | 11.0639 s | 11.5856 s | +4.71% |
| user CPU | 9.8744 s | 10.1638 s | +2.93% |
| system CPU | 1.2720 s | 1.3288 s | +4.47% |
| 峰值 RSS | 3,376,568 KiB | 3,334,000 KiB | −41.57 MiB / −1.26% |

本机 wall 噪声明显：before 范围 10.427–12.128 s，after 10.549–11.790 s。
如实记录本组中位数略慢，不能据此宣称加速，也不将 +4.71% 当成精确、可移植的
退化常数。这里完成的是身份、回收、版本与边界契约，不是高收益 CPU 优化。

| 独立 native 分配诊断 | before | after | 变化 |
| --- | ---: | ---: | ---: |
| 分配 API 事件（含 realloc） | 195,962,909.5 | 196,063,602 | +0.051% |
| 请求字节 | 14,074,718,107 | 13,936,873,648 | −137,844,459 B / −0.979% |
| realloc 事件 | 14,468,426 | 14,726,094 | +1.781% |

两对交错独立诊断样本；半个事件来自两个整数样本中位数取平均，不是实际出现
分数次调用。请求字节减少不等于活跃堆减少；事件数基本持平且 realloc 增加，
因此不宣称本次显著减少了分配次数。

| 独立粗粒度阶段归因 | before | after |
| --- | ---: | ---: |
| pipeline.analyze | 0.9681 s | 0.9703 s |
| pipeline.evaluate | 4.0598 s | 4.2620 s |
| ssg.bodies | 2.3123 s | 2.2252 s |

阶段数据来自另外两对带计时探针的完整构建，保留调用次数、父子边和模块统计。
`ssg.bodies` 包含正文相关工作，并非纯 HTML renderer；这些 inclusive 阶段不能
直接相加，也不能拿探针进程的总耗时替代 clean wall。求值阶段有可见额外成本，
但这组归因不能单独证明弱表、来源检查或某个容器各自贡献了多少。

最终 10 次 clean、4 次分配诊断、4 次阶段归因均成功，全部 78,004 项产物在每轮
计时之外校验一致；完整输入、普通/插桩源码及二进制哈希复核通过。
`integrity-check.json`、`validation/` 和 `reproducer-inventory.json` 保存最终校验。

## 结论

完整 32 位 SourceId 和影响面迁移已验收。新实现没有永久来源名池，也没有把
SourceId 当作正文版本或权限；Rust API、源码快照、缓存和 wire 边界一起迁移。
本轮收益是更紧凑的来源表示、约 41.6 MiB 的整站峰值 RSS 降幅，以及明确的
生命周期/修订正确性；没有验证出整站 CPU 加速。不自动提交，也不继续推进 DefId。

## 后续归因补充

后续完整 profile 已确认主要回退来自校验器双重遍历 literal children，而不是
32 位 ID 或弱表回收不可避免的成本。仅移除重复入口、保留校验的外部诊断副本
已取得改善；该修复尚未合并生产，性能收尾待处理。
详见同目录的 [完整成本归因](2026-11-07-sourceid-cost-profile.md)。
