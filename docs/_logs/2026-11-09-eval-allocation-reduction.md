# 求值路径减少无用分配

## 结果

基于 9a3c5f7，在独立 worktree 的 perf/reduce-eval-allocations 分支实施三项局部优化。
三项合用的完整站点构建 wall 中位数由 8.501 秒降至 8.158 秒；五组交错配对的耗时变化中位数为 -3.99%。
单次父进程 malloc 由 132,729,790 次降至 108,697,813 次，减少 24,031,977 次，约 18.1%。
峰值 RSS 基本不变，不能将分配次数减少解读为峰值内存下降。

默认特性回归 836 项、全特性回归 846 项全部通过；两组各 84 个测试套件，失败及忽略均为 0。
所有 39 次完整站点构建均核对 78,004 项产物的长度及 SHA-256，一致；四组分析／求值 JSON 对照也一致。
这不是增量编辑、LSP 或 Worker 的性能保证，也没有测量优化后的 CPU 热点重排。

## 实现

### 只为 Code 调用创建输入作用域

crates/notist-eval/src/eval.rs：execute 的输入 Scope 只在 code 条件成立时创建和装配，native 调用不再创建空共享 BTreeMap。
仍在原有位置求值实参、解析 implementation 和装配 Code 输入，不把它们移到调用深度检查之后，以免改变错误顺序。
Code 执行分支消费所准备的作用域；若上游 registry 元数据未标为 Code，仍保留原有空作用域执行行为。

### 节点直接构造最终 payload

crates/notist-eval/src/eval.rs：item_with_content 集中复制 attrs、origin、level 和 span，接收最终 ContentItem。
native 构造、Element 物化、错误节点与带属性的挂载 Group 均直接填入最终 fields／children／instance，不再先创建节点壳再覆盖。
crates/notist-eval/src/test_support.rs 的测试辅助入口同步适配，保留原有 item_shell 验证接口。

新增测试验证最终 children 的存储复用、源节点不变，以及属性、来源、层级、范围和错误节点 payload。
没有引入结果缓存，没有取消重复挂载的独立执行，没有修改预算或来源校验。

### 字段键借用查找，只在插入时创建 Text

crates/notist-core/src/shared.rs：Dict::insert 使用 AsRef<str> 查找键；仅新增键时从 &str 创建最终 Text，已有键更新不创建临时 String。
保留首次位置／最后值、重复 wire 条目、COW、索引阈值、metrics 失效及保守容量预算。
crates/notist-core/src/item.rs 和 expr.rs 的 with_field 接口同步接受借用字符串键。

新增测试覆盖小字典和索引字典、更新时保留已有键存储、COW 后旧值／旧 metrics 不变、插入顺序及 String／Text／自定义借用键。
现有 Unicode、重复键序列化和预算回归也通过。

Rust API 注意：这三个键入口的泛型约束由 Into<String> 改为 AsRef<str>。
String、&String、&str、Text 等正常字符串输入继续可用；外部自定义的仅实现 Into<String> 的类型需要显式转换为 String 或实现 AsRef<str>。
语言语法、输出格式及依赖没有变化；没有替换生产 allocator，也没有改用 EcoVec／EcoString。

## 普通构建 A/B

同一冻结站点：51,739 页，78,004 项输出，共 451,812,631 字节。
基线普通 release 二进制及源码来自已校验的 9a3c5f7 证据；每个候选分别冻结源码、使用独立 Cargo target 构建。
均为无探针 release，相同工具链与构建配置。每组轮换 before／candidate 的执行顺序，不与本轮编译或测试任务并行测量。
合并候选五对，单项各三对；五个版本分别预热。以下基线中位数只取该候选相邻配对的基线，不混用全部基线。

| 候选 | 对数 | 对应基线 wall | 候选 wall | 配对耗时变化中位数 |
| --- | ---: | ---: | ---: | ---: |
| 仅作用域 | 3 | 8.537 s | 8.306 s | -2.32% |
| 仅节点 | 3 | 8.365 s | 8.190 s | -2.21% |
| 仅字段键 | 3 | 8.337 s | 8.318 s | -0.22% |
| 三项合用 | 5 | 8.501 s | 8.158 s | -3.99% |

各项变化不能相加：源码优化有组合效应，各组还存在机器状态和执行时间差异。
仅字段键三对的耗时变化为 -0.22%、-1.09%、+5.66%，没有稳定 wall 加速证据；保留它的依据是消除明确的中间分配，而非宣称该项稳定提速。
合并候选五对均更快，耗时变化范围约 -4.68%～-3.77%；按五对比值重采样的局部 bootstrap 区间也落在该范围内，但样本很少，不代表跨机器置信保证。

合并候选对应的 user CPU 中位数由 7.746 秒降至 7.393 秒；system CPU 为 1.012／1.032 秒。
maxrss 中位数为 2,836,980／2,837,272 KiB，约 2.71 GiB，差异处于噪声量级。
本轮全是已有产物的完整重建，发布主要核对内容是否变化；不能外推首次完整写盘收益。

## 独立分配对照

使用相同 LD_PRELOAD 诊断包装器调用 glibc 原实现，仅统计目标 Rust 父进程的成功分配 API；扩展子进程不计入。
这些运行有统计与概率调用栈采集开销，其 wall 不进入上表。

| 候选 | malloc 次数 | 相对基线减少 |
| --- | ---: | ---: |
| 基线 | 132,729,790 | — |
| 仅作用域 | 126,708,242 | 6,021,548（4.54%） |
| 仅节点 | 120,380,867 | 12,348,923（9.30%） |
| 仅字段键 | 127,068,284 | 5,661,506（4.27%） |
| 三项合用 | 108,697,813 | 24,031,977（18.11%） |

此次 malloc 次数的三项减少量正好相加等于合并减少量，但耗时不具有该可加性。
五个版本的 calloc 均为 37 次、realloc 均为 14,726,619 次，说明没有解决路径增长等剩余热点。
累计 malloc 次数不是同时存活对象数量，因此不能据此宣称 RSS 相应下降。

首次分配诊断基线向空 dist 发布，与随后候选的产物比较工作不同，未用于以上对照。
该原始记录保留；在已有完整产物后另跑基线，比较结果保存为 allocation-counts-comparable.json。
所有用于对照的分配运行记录丢失均为 0。

## 验证与复现

测试采用 cargo test --release --locked --offline --workspace --lib --tests，分别使用默认与 --all-features，测试线程 2。
编译初期并行度 2；磁盘空间不足导致链接失败后改为 1。恢复时补齐 CLI 夹具要求的 worktree 内 target 目录，再完整重跑，未改动测试预期。
rustfmt --check 及 git diff --check 通过。

四组 JSON 对照覆盖词法捕获与重复挂载、失败诊断、泛型调用及数据读取；两侧使用同一夹具路径。
性能、分配、输入和输出的完整冻结证据位于 /tmp/opencode/notist-reduce-eval-allocations-20261109。
其中 run.py 保存复现步骤；manifest.json 记录源码清单、二进制摘要、工具链及变更文件；production.patch 保存生产代码与测试变更。
clean-samples.json、clean-summary.json、allocation-counts-comparable.json、semantic.json、tests.json 和 logs 保留原始结果。

run.py 的 prepare、isolate、build、measure、allocations、allocation_baseline、semantic、verify 接收相同 --repo 和 --reference。
本次 --reference 为 /tmp/opencode/notist-hotspots-9a3c5f7-20261109；先执行分配与语义校验，待回归编译结束后执行普通计时，最后再次 verify。
如复现时将 run.py 放在新的证据目录，按 all 执行即可；它拒绝覆盖已有冻结证据。
