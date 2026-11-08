# DefId / ModuleId：可回收的逻辑定义身份

## 完整性复核：当前不是全链路迁移

用户进一步询问完整性后重新核对：本报告的测试与性能数字只验收当前的共享名称拥有
句柄和部分数字索引优化，不能称为“DefId / ModuleId 全影响面重构完成”。此前将 P2
标记为完成过早，已重新打开；生产代码仍未提交。

尚未迁移的内部通路包括：

- FunctionInstance/Invocation 等仍携带 FunctionId 名称拥有句柄，没有将全部已解析
  IR 的裸 DefId 与名称拥有快照分离。
- CodeProgram 的 modules/scopes、CodeModule 的 bodies，以及 body 查找仍使用模块名/
  函数名字符串；已解析的 ImportBinding::Module 也仍保存 String。
- HtmlRegistry 仍按 FunctionId 的词典序比较查找，没有将内部数字索引与外部稳定排序
  拆开。相关宿主、链接和显示边界还需随完整方案一起迁移、验证。

解析输入的名字、导入别名、外部显示名、wire 名称与稳定排序应当继续保留文字，不能
为了“迁移彻底”盲目删除。实例格式化键/InstanceId 和 BindingId 从一开始就明确留给
独立任务，不算这轮必须偷偷合并的工作。完成上述边界梳理和迁移后，须重新整体验收；
本轮性能数字仍有效，但只代表当前局部版本。

## 范围

本轮起点为 `6a8e479`，生产内核对应已验收的 SourceId 修复 `b026bcc`。
先提交用户要求的现有文档改动，再实现定义身份；不改 consumer、依赖锁、默认额度，
不混入 BindingId、InstanceId、结构化特化键或调用协议重写。

## 身份和拥有关系

- `DefId` 和 `ModuleId` 都是 `NonZeroU32` 新类型，均为 4 字节，可复制；
  `Option` 也为 4 字节。分别使用单调计数器，退休后不重用，溢出检查不回绕。
- `ModuleRef` 是拥有模块名称的句柄。
- 现有公共名 `FunctionId` 保留，但表示共享的、不可变的名称拥有句柄，**不是 4 字节
  的裸 ID**。其 `def_id()` / `module_id()` 返回上面的数字身份。调用、输出内容及配置
  可以独立保留名称，不需要借助全局反向查找或保活整个 Registry。
- 这轮没有声称“所有 IR 中的函数引用都只占 4 字节”。压缩的是数字索引键，及从每份
  函数引用拥有两个 String 改为共享名称拥有关系，消除克隆时的字符串分配。
- 名称通过弱表登记；最后一个名称拥有者释放时，删除对应的函数键。模块的最后一个
  拥有者释放时删除模块键。弱表不保留正文、签名、实现或不可回收的任意名称池。
  HashMap 可保留高水位容量，但空模块的函数子表被删除。

这里的逻辑身份是当前语言已有的“规范模块身份 + 声明名”，而非裸拼写。不同模块里
同名函数不相等，import 别名不改变定义身份。它不表示版本：两个 Context 中相同规范
模块/函数可以具有相同逻辑 ID、不同函数体或签名。禁止据此跨 Context 复用实现。

`try_new` 提供耗尽错误；保留的便利构造函数 `new` 在身份空间耗尽时显式失败，不回绕。
反序列化使用可失败入口。与已有 API 不同，不能再单独修改 `function.id.name`；
应替换整个名称句柄，避免文字与数字身份脱节。

## 数字键的使用

- Registry 保留名字入口用于加载/解析，同时增加按 DefId 的定义索引、按 ModuleId
  的模块索引。索引指向当前 Registry 的不可变模块快照，包括空模块；COW 安装不影响
  旧 Registry。不是全局实现缓存。
- 求值器的 evaluation-local InstanceCache 改用 DefId 键，仍检查编译期参数；每组
  缓存条目显式保留名称拥有者，不能留下无拥有关系的裸 ID。
- runtime replacement 比较使用 FunctionId 的数字身份相等；规则顺序、实际目标、
  comptime 不使用 runtime replacement 的规则保持不变。
- builtin 构造节点从既有 builtin Registry 取得共享名称句柄，避免每次投影重新构造
  两个字符串或重新登记。Registry 的初始化不依赖这个投影。

实例格式化键 `FunctionInstance::key()`、浮点 bit-exact 规则和 Data comptime 的
权威回退路径未改；本轮不靠省掉检查或扩大缓存获取收益。

## 外部行为

FunctionId 的 JSON 仍为 `{module, name}`，Debug/Display 保持原有拼写；裸 ID
不实现 serde。公开有序集合仍按模块名、函数名的字典顺序排列，不能按进程编号排序。
名称在 Registry 或 Context 释放后仍可显示和序列化。

## 回归与验收

新增 5 项核心测试和 5 项集成测试，覆盖 32 位/超过 16 位容量、弱表键回收、退休不复用、
并发登记、耗尽不回绕、跨 Registry 不同修订、空模块、旧快照不受安装影响、逃逸名称、
JSON 和稳定排序。真实编译/求值测试交错使用相同逻辑 ID、不同函数体的两个 Context，
确认没有复用另一 Context 的 checked implementation。

- 全特性 workspace：804 项通过。
- 无默认特性 workspace：788 项通过。
- 独立 core/eval/pipeline 无默认特性：164 项通过。
- Python 工具：64 项通过。

正式 clean 与独立分配 ledger 使用 `tools/profile_definition_identity.py`，源码、
二进制、输入、完整产物清单分别校验。旧版本直接复用已验收 SourceId 修复的无插桩
release，不使用带重复遍历缺陷的版本或永久池原型。新版本从本轮冻结源码构建。

证据目录：`/tmp/opencode/notist-definition-identity-20261108`。

## 正式整站 A/B

两版本分别预热一次，再做 8 对交错完整构建。所有样本均保留，不删除 I/O 异常轮次。
分配诊断在 clean 完成后独立构建、运行，不把其耗时用于加速比。

| 同一轮 clean 中位数 | 已修复 SourceId 基线 | DefId / ModuleId |
| --- | ---: | ---: |
| wall | 10.1029 s | 8.5765 s |
| user CPU | 8.6383 s | 7.6977 s |
| system CPU | 1.1777 s | 1.0729 s |
| 峰值 RSS | 3,319,774 KiB | 2,800,542 KiB |

配对 wall 比值中位数 **−14.59%**，8 对均更快；user CPU 中位数 **−10.89%**。
峰值 RSS 减少 **507.06 MiB**。这里的 baseline 已有 SourceId 互斥遍历修复，
不能把该收益再计入本轮，也不与上一轮的绝对秒数拼成一个加速比。

本轮多次出现 I/O/等待波动：例如 before 第 4 对为 11.201 s、26,998 次 voluntary
context switches，第 6 对为 10.937 s、27,250 次。after 也有 major faults 和等待。
因此 −14.59% 是这批整站 wall 的观察，不应把全部差值都叫作纯 CPU 收益；user CPU
与独立分配统计提供另一条同向证据。这不是跨机器的性能保证，没有挑选最有利轮次。

## 独立 Rust 分配和内存 ledger

两边都在外部诊断副本中使用相同的 System allocator 计数探针，原始正常版二进制不变。
计数是 Rust 请求布局，不包含 Python 扩展的分配；普通分配含 alloc/alloc_zeroed，
realloc 请求字节计完整新布局，不等于实际复制字节或同时驻留内存。

| 精确计数 | SourceId 基线 | 本轮 |
| --- | ---: | ---: |
| 普通分配次数 | 181,337,444 | 133,664,853 |
| realloc 次数 | 14,776,801 | 14,776,801 |
| 普通分配累计请求字节 | 10,452,775,210 | 9,473,565,265 |
| realloc 累计请求字节 | 3,300,140,301 | 3,283,395,745 |
| requested-live 峰值 | 2,691,860,335 | 2,352,500,342 |
| CLI 局部析构后 requested-live | 2,953,498 | 2,954,353 |

普通分配减少 **47,672,591 次（−26.29%）**，realloc 次数不增加。请求堆峰值下降
**323.64 MiB**，尾端只多 855 字节。弱表键的具体释放由单元测试验证；这些尾端数字
不支持永久保留整站任意名称，但也不能外推为已做过长驻 LSP 数千轮编辑压测。

本机布局进一步实测：Node **128 → 104 B**，Invocation **184 → 144 B**；
TypedExpr 仍为 136 B。FunctionInstance 为 32 B，FunctionId/ModuleRef 两种拥有句柄
各为 8 B，裸 DefId/ModuleId 各为 4 B。这些不是稳定 ABI 承诺。baseline 布局使用归档的
完整 SourceId 布局二进制，未使用被回退永久池原型的候选布局。

RSS 与 requested-live 的变化口径不同，不能把两者之差全归给某个字段或分配器碎片。
本轮没有再做新的 CPU 采样；分配收益与先前 FunctionId clone/new 调用栈归因同向，
但不把累计分配次数直接换算成某个函数的 CPU 占比。

## 完整性与交付状态

包含预热和两次 ledger 的 **20 次完整构建**，每次都在计时外逐项核对全部
**78,004 项 / 451,812,631 字节**产物，全部一致。冻结源码、二进制和原输入清单
也已复核；本轮仅 10 个 Rust 文件变化，没有 Cargo/锁/默认预算改动。

用户同期继续编辑的其它文档未纳入这轮源码改造。新实现和本报告保持未提交，
未推送；不自动开始下一项 InstanceId 或 BindingId。

复现时使用新的外部目录，避免覆盖证据：

```sh
python3 tools/profile_definition_identity.py prepare --out /tmp/opencode/definition-identity-REPRO
python3 tools/profile_definition_identity.py build --out /tmp/opencode/definition-identity-REPRO
python3 tools/profile_definition_identity.py measure --out /tmp/opencode/definition-identity-REPRO
python3 tools/profile_definition_identity.py ledgers --out /tmp/opencode/definition-identity-REPRO
python3 tools/profile_definition_identity.py verify --out /tmp/opencode/definition-identity-REPRO
```

`--reference` 默认指向已验收 SourceId 修复的证据目录，`--target` 只选择外部编译缓存。
实现补丁、改动文件清单、完整测试日志、clean 样本、ledger、布局命令及二进制均归档。
