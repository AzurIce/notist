# 求值器实例复用与签名 COW（2026-11-04）

本轮开始落实 [求值器 A/B](2026-11-04-evaluator-ab-profile.md) 中证据较稳定的两项改进。没有同时落地单独计时区间跨零的父链 Scope 或 present Vec 复用，也不把此前四项组合的 8.53% 当作本次实现的收益。

## 实现边界

### 1. Invocation 签名：拥有型共享、写时复制

`Invocation.definition` 从 `Box<FunctionDef>` 改为 `notist_core::shared::Shared<FunctionDef>`。`Shared<T>` 内部持有 `Arc<T>`：

- clone 只增加引用，不复制 FunctionDef 的字符串、参数和默认值容器；
- 读取通过 Deref/AsRef；
- 可变访问通过 `Arc::make_mut`：仍有其它所有者时才复制，唯一所有者直接修改；
- `into_inner()` 在唯一持有时移出，共享时复制；
- PartialEq、Debug、serde 仍观察内部值，不使用地址作为语言身份或相等性的捷径，尤其不把同地址 NaN 判为相等；
- 不启用新的依赖特性，也不改变 CheckedProgram 的所有权或公开 wire 数据。

这里不使用 `std::borrow::Cow<'a, FunctionDef>`。调用、捕获、Analysis 和跨模块对象需要独立拥有自己的生命周期；借用型 Cow 会把生命周期限制传播到整个 IR，而且拥有值的普通 Cow clone 并不自动共享其存储。

现有 Text、Array、Dict 已经使用共享存储；本次不是重新包装它们。整棵树在挂载时仍要独立更新来源和边界，随手加一个树级 COW 可能在每次首次修改时照样触发大范围复制，所以不把签名上的收益推广到整棵树。

共享的是**签名快照**，不是执行结果。每次挂载仍独立执行；Item 仍是权威树，直接修改一个调用的契约仍由求值器读取，不改为从缓存中的另一个签名覆盖它。

### 2. 成功实例：本次 evaluator 的有界查找加速

新增私有 `InstanceCache`，由每个 Evaluator 自己拥有：

- 按完整 FunctionId 分组，组内比较结构化 Type 参数；无 comptime 参数也可复用；
- 只记录成功、已完成检查的 InstanceRecord；诊断和 checking 中的记录不记录；
- 任何 Data comptime 值均保留原规范 key 路径，包括普通数字、字符串、浮点负零和 NaN，而不只排除 Data view；
- 最多 256 个条目、每个函数最多 16 个特化；满了仅回退 CheckedProgram，不改变语言实例数、深度、步骤或资源预算；
- 目标仍先经过当前 Context 的 transform，缓存实际目标；不会跨 Context、输入批次、另一次求值或 serve 重建复用；
- 缓存的只是已经查到的实现记录，仍逐次进行参数检查、执行、读取、依赖登记、挂载和预算记账。

这是前次线性 Vec 实验的正式、有界实现，不能直接继承那个原型的耗时结论，需要重新验收。

## Rust API 迁移

序列化值不变，不等于 Rust 源码完全兼容。具体调整如下：

```rust
// 构造 Invocation 字段：此前使用 Box::new(definition)。
definition: definition.into(),

// 已经拥有 Box<FunctionDef> 时也可以转换。
definition: boxed_definition.into(),

// 原有字段访问与直接修改继续可用；修改共享快照时自动分离。
call.definition.parameters[0].constraint = ValueConstraint::PositiveInt;

// 需要移出整个 FunctionDef，而非借用时：
let definition = call.definition.into_inner();
```

持有 `Box<FunctionDef>` 的显式类型注解、依赖 Box 专有方法或直接从解引用处移出字段的外部代码需要迁移。没有把 HTML component 的另一个 Box 字段无条件改为 Shared，也没有给所有 Item children 套共享层。

## 验证协议

证据目录：`/tmp/opencode/notist-runtime-cow-20261104/`。`sources/before` 冻结实现前的完整工作区（包括原有未提交修改），`sources/after` 仅覆盖五个生产文件；并非拿当前工作区直接与干净 HEAD 做比较。

工具为 `tools/profile_runtime_cow.py`，完整脚本另存证据目录的 `reproducer.py`。源文件、输入、二进制、脚本都有 SHA-256 指纹。两版用相同 release 配置和 Cargo.lock，workspace 默认与全特性测试分别验证；测试单线程执行以避免既有模块共享指针断言的并行不稳定性。

首次 workspace 基线测试有两项 CLI fixture 失败，原因是其 `tempdir_in` 要求源码副本内存在 `target/`，而 Cargo 产物使用外部目录。补建空目录后重跑；未改生产文件或断言，初次失败日志及脚本保留在证据目录。这项环境准备发生在全部正式计时之前。

默认与全特性测试产物同时保留曾令磁盘在基线全特性链接时写满。失败日志保留，随后在各测试模式间仅清理本次外部 target 中的 workspace 构建产物；成功的测试日志、测量用普通二进制和不可变输入均保留。这不改变编译优化选项或正式测量样本。

全量验收沿用冻结的完整 IceXIV：51,739 页、78,004 个输出文件。先完成初次发布，再各一次 warmup，之后六轮 before/after 交替顺序配对计时，覆盖构建、现有输出完整比较和进程退出；每轮结束后核对所有输出哈希。另测两轮 Rust PID VmRSS，不把带 RSS 采样的耗时混入主组；保留 major fault 等信息，不事后挑掉慢样本。

四组补充 fixture 比较完整 analysis/evaluation JSON，而非只比较最终 HTML，覆盖捕获与重复挂载、恢复诊断、泛型和 Data 读取。Shared 单测还验证唯一写入不 clone、共享写入仅首次 clone、两条 into_inner 分支、无 Clone 约束的共享克隆、NaN 相等性及 Box/Shared 签名的 wire 一致性。

## 测试结果

- before workspace 默认 **726 项**、全特性 **735 项**通过。
- after workspace 默认 **733 项**、全特性 **742 项**通过。
- 各模式均为 74 个 test target，零失败、零编译警告；新加的 7 项回归覆盖 Shared 的 COW／wire、缓存身份／回退／上限以及签名修改后的权威性。
- 初轮核心／管线／根库默认测试 **507 项**通过；验收工具自身的三个 Python 单测验证五文件覆盖限制、配对统计及断点续跑／测试产物清理策略。
- after 核心／管线无默认特性 **129 项、14 个 test target**通过，不依赖 workspace 统一开启 serde 才能编译。
- 4 组 fixture × 2 版完整 analysis/evaluation JSON 相同，恢复案例保留预期的 inner failure，成功案例无求值诊断。
- **19 次全量构建的 78,004 文件输出哈希逐次一致**：初次发布 1 次、warmup 2 次、正式计时 12 次、独立 RSS 4 次。消费者的站点检查验证 51,225 个详情页、513 个目录页及全部本地 HTML 链接通过。

## 无插桩性能验收

这批是当前 dirty 工作区的 before/after，**不是**把旧实验的秒数与新实现混算。正式二进制没有 profiling 功能、全局分配观察器或高频探针；每次调用是一个新进程，现有输出比较及退出都在 wall 内。

| 六轮普通构建 | before | after |
| --- | ---: | ---: |
| wall 中位数 | **15.180s** | **14.235s** |
| user 中位数 | 13.664s | 12.768s |
| system 中位数 | 1.784s | 1.713s |

两个 wall 中位数少 **0.946s、6.23%**；逐轮先计算 `(before-after)/before`，再取中位数，则为 **6.38%**。六轮均更快，配对降幅依次为 `9.05 / 6.45 / 6.72 / 5.83 / 6.30 / 6.16%`。六样本配对中位数的描述性 bootstrap 95% 区间为 **5.99–7.88%**，不证明其它机器或工作负载上的普遍区间。

正式组记录 28 次 major fault，其中首对 before/after 为 8/14 次；没有剔除首对较慢的基线，也没有删除其它样本。CPU 频率与系统任务未完全锁定。输出哈希核对、编译、额外语义测试和 RSS 采样均不计入这 12 个主测进程。

### RSS 未改善

25ms Rust PID VmRSS 独立采样的两次峰值：

- before：**6.4822 / 6.4827 GiB**；
- after：**6.4945 / 6.4946 GiB**。

约多 12MiB、量级基本不变；本轮不声称省峰值内存，也没有重新测量整进程分配流量，不能沿用旧四项组合的 −18.18% 请求字节数字。签名共享不改变最终树规模，运行时局部缓存也仍持有少量记录，生命周期及紧凑树问题依然存在。

`summary.json` 保留主测统计，`samples.json` 保留 warmup 与正式原始样本，`rss.json` 及逐次轨迹保留内存观测，`semantic.json` 与逐次 JSON 保留语义对照。源码、普通二进制和输入指纹最终复核；只清理本次外部 Cargo target 的可重建编译缓存，保留证据快照、日志、二进制、完整输入和输出。

## 后续范围

边界元数据与整站树生命周期仍是独立待办：先细分 item_shell 的字段成本，再单变量测 FunctionId／来源共享、持久来源链和廉价恢复 checkpoint。本次不会用减少分配流量声称峰值内存下降，也不会靠取消挂载或把销毁移出计时制造收益。
