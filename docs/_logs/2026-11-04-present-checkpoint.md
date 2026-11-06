# present 的最小恢复 checkpoint（2026-11-04）

接续 [调用恢复所有权](2026-11-04-boundary-recovery.md)。本批只替换 present 在检查兄弟节点累计额度前建立的完整 Item shell 备份，不改迭代器／Vec 策略、不改公开 Item／Origin、不改变挂载执行或全站树生命周期。

## 1. 哪些信息真的需要备份

原实现对每个输入节点先做 `item_shell(&node)`，再调用 present_node，最后检查 `summary.nodes + added.nodes > limit`。即使从未溢出，也会复制身份和整条来源链。

当前求值器的成功路径有以下约束：

- 普通节点只改 children；重复挂载还会改 evaluation。ctor、instance、origin、attrs、span、level 不变。
- 调用（包括 synthetic replacement）成功后会覆盖 fields、target 和 evaluation；来源扩展施加到子树，不改保留的调用边界。
- 自身失败返回零个正常节点，因此不会让已经满足额度的兄弟节点累计值溢出。
- error 会重新设置 evaluation、清空 pending 和 children；这些无需额外备份。

据此，PresentCheckpoint 只在实际调用路径上保存原 fields／target。Dict 已共享，因此 fields clone 没有新分配；target 仍按值复制，以正确支持宿主提供的旧 target，不能擅自假设它总是 None。

还有一个更早的跳过条件：每个 present_node 的正常节点数本就受完整 limit 限制。前面没有正常兄弟时，累计溢出不可能发生，所以第一项（或前面全是零节点错误的项）连 fields／target 都不用备份。

**没有把传给 present_node 的 limit 改成剩余额度。** 后者可能改变在哪一层恢复、先执行了哪些读取、错误顺序和保留的来源，不能作为等价优化混入本次。

溢出时复用已经拥有的节点，只把调用覆盖过的 fields／target 恢复，然后仍调用原 error；不把成功执行的新字段或 target 当作原边界。没有保存另一棵语义树，也不需要新的 Arc/COW 包装层。

## 2. 旧实现作递归对照

原来的完整 shell 恢复循环保留为 `#[cfg(test)] present_reference`。测试线程的局部开关使所有递归 present 都走原算法；普通 CLI 不链接该方法、开关或测试计数器。

新增差分回归覆盖：

- 7 个场景：兄弟调用扩张、children 重复传递、循环、带内部 panic 的恢复、重复内容挂载、表格结构、强制 synthetic replacement；
- 2 个 frontend；
- content_nodes 为 0–63，分别执行正常额度与受限 steps／iterations／call_depth；
- 合计 **1,792 组前后求值比较**，核对完整 Item 树、诊断顺序／来源、依赖和数据源码。

测试给原树附加字段、注解、来源扩展，并特意移除调用的 instance、设一个旧 target，验证 error 无法重建身份时仍恢复旧 target。另有计数断言要求确实发生了成功调用后的 checkpoint 恢复，而不只是覆盖未溢出的情况。独立单测验证首项可跳过备份及原字段恢复。

准备时修正过新测试 fixture 的 extension 参数类型和 text 命名参数；失败日志及源码草稿保留，均发生在正式验收之前，没有修改旧测试断言。

## 3. 测量协议

证据目录：`/tmp/opencode/notist-present-checkpoint-20261104/`。before 与上一批已验收 after 逐文件相同；沿用该相同源码的默认 734／全特性 743 测试日志，不重复声称执行了基线测试。普通 before／after 二进制在本批各自重建，after 只覆盖 eval.rs。

`tools/profile_present_checkpoint.py` 复用既有冻结、字段观察和普通 A/B 协议；记录 present_recovery 原 shell 与 checkpoint 的 fields／target 克隆。观察器使用原 System allocator、固定容量记账，不把它的耗时当无插桩加速。原 item_shell 的 bind／capture 场景单独保留，用来确认没有少处理其它工作。

本批基线中 present 恢复仍有 **8,389,713 次 shell、51,156,773 次分配事件、1,728,896,824 请求字节**。来源字符串包含本次证据目录路径，所以请求字节不同于上一目录，不跨批混算。

普通组先 warmup，之后六轮交替顺序的整进程构建；每轮结束后核对全部 78,004 个文件。另行检查四组完整语义 JSON、站点本地链接，并采样 Rust PID RSS；不改变消费者、锁、授权、资源请求或逻辑额度。

## 4. 验收结果

### 分配归因

原 present_recovery 的 **8,389,713 次 shell、51,156,773 次分配、1,728,896,824 请求字节全部消除**。新 checkpoint 实际保留 fields／target **2,006,497 次**，本负载这些克隆均无新堆分配。它仍支持宿主提供非空旧 target，不能推广成任何输入都零分配。

bind 与 capture 的每个字段记录在前后完全一致。包括 checkpoint 在内的已观察边界分配从 **64,246,960 → 13,090,187 次**，请求字节从 **2,113,865,487 → 384,968,663**；剩余真实 shell 为 **4,277,854 次**。这些是边界范围的流量，不是整进程总分配或 RSS。

### 无插桩完整构建

| 六轮中位数 | before | after |
| --- | ---: | ---: |
| wall | **15.734s** | **14.628s** |
| user | 13.829s | 12.785s |
| system | 2.134s | 2.123s |

两个 wall 中位数少 **1.106s**。逐轮配对的耗时降幅为 `7.40 / 8.28 / 7.18 / 6.28 / 3.44 / 8.96%`，中位数 **7.29%**，六轮都更快；六样本配对中位数的描述性 bootstrap 95% 区间为 **4.86–8.62%**。

本批系统噪声较大：正式进程发生 14 次 major fault，非自愿上下文切换每轮约 820–3,824 次，CPU 频率及其它任务未锁定。所有正式样本保留。两版字段探针 wall 为 20.361s／23.644s，不用它们的差值声称加速。

本批来源路径较长，系统环境也与前一批不同，绝对 wall 比前一批高；**只用本批同目录、同输入的配对比较，不把本次 after 与上次 after 相减来判断回退，也不把三批百分比直接相加**。

### 测试、输出和内存

- after workspace 默认 **736 项**、全特性 **745 项**通过，均为 74 个 test target、零编译警告；核心／管线无默认特性 **132 项**通过。
- 新增 2 项回归；其中差分测试的 1,792 组比较在各测试配置中重复执行。观察器 Rust 单测 1 项、新冻结／插桩工具 Python 单测 1 项通过，既有边界工具的两项测试继续通过。
- **21 次完整构建**逐次核对全部 **78,004 个输出文件 SHA-256**，全部一致；四组完整 analysis/evaluation JSON 前后一致。消费者的 51,225 个详情页、513 个目录页与本地 HTML 链接检查通过。
- 两次独立 Rust PID RSS 峰值：before **6.5621 / 6.5617 GiB**，after **6.5735 / 6.5732 GiB**，基本不变；不声称峰值内存改善。

源码、输入、二进制、脚本最终指纹复核，生产文件相对上一批只有 eval.rs 改动。证据目录保留完整源码、输入／输出、二进制和日志；仅清理本批外部 Cargo target 中的可重建产物。所有修改未提交。

## 后续范围

checkpoint 已通过验收，剩余 shell 来自 bind／capture 的真实结构构造，之后按这些字段和来源传播的成本决定是否做共享来源／身份。全站树峰值、跨轮增量和源码位置的持久表示仍是其它专项，不能由短命分配减少推断已解决。
