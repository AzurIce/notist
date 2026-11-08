# DefId / ModuleId：完整内部迁移验收

## 范围与身份边界

本轮补全此前局部版本遗漏的内部通路。旧报告
`2026-11-08-definition-identity.md` 的数字仍只对应局部版本，不改写为完整结果。
本轮基线仍是已验收 SourceId 修复 `b026bcc`，不是有重复遍历缺陷的旧版本。

- `DefId` / `ModuleId` 为私有构造的 `NonZeroU32`，裸引用与 Option 均为 4 字节。
  弱登记表回收名字与空的函数子表，退休编号不重用，耗尽不回绕。
- `FunctionId` / `ModuleRef` 是名称拥有句柄，不冒充 32 位 ID。身份表示规范模块中的
  逻辑声明，不表示修订、签名一致、资源授权或跨 Context 的实现缓存有效性。
- `Definitions` 显式保留名称；`Sources` 同时携带源码修订与定义名称快照，伴随
  Analysis、CheckedProgram、Evaluation、Located 及扩展上下文逃逸。

## 完整内部通路

- Invocation、RExpr、ContentItem 的实例引用改为 `ResolvedInstance { function: DefId,
  arguments }`；ExtensionCtor 与 Node.function_id 使用裸 DefId。
- ImportBinding 保存 ModuleId / DefId；ModuleScope 保留所选导出的名称拥有关系。
  缓存仍只跟踪查询到的导出，不因无关导出变化而失效。
- Registry 的权威定义／模块索引、CodeProgram 的模块／作用域表、CodeModule 的函数体
  表均为数字键；文字模块索引只映射到 ModuleId，不复制实现记录。
- 检查器当前模块、Context 根模块、宿主加载图及验证集合使用 ModuleId。宿主使用
  resolved 导入／链接 API，不在内部把数字转回名字再查找。
- HtmlRegistry 的目标／绑定索引和 ExtensionPlan 的执行映射使用数字身份；组件标签、
  显示、诊断与外部列表通过明确的文字投影保持原来的词典序。
- DefinitionModule 拥有 ModuleRef；CodeModule.module_id 从公开声明派生。模块 COW
  重定位在链接时重建函数体索引，不污染原编译对象。
- 查询、dump、JSON/CST、LSP、命令扩展与 wire 显式接收名称快照。裸 ID 不实现 serde，
  wire 仍输出规范 module/name。decode 拒绝未知名字；显式 import 返回新拥有关系。
  输入 DTO 验证前只含文字，未知输入不会偷偷创建身份。
- 外来作用域 ID、调用实例／声明身份不一致、checked record 冒充另一身份均明确失败。
  动态编译和宿主装配使用可失败分配入口，不以隐藏全局反查补齐拥有关系。

FunctionInstance 保留为外部选择器；实例字符串键／InstanceId、BindingId、FieldId、
调用协议与全站内存生命周期改造均未混入。没有修改 consumer、Cargo.lock 或默认预算，
没有重启用户取消的采样服务器；没有自动提交或推送。

## 正确性验收

冻结候选后重新执行：

- workspace 全特性：812 项通过。
- workspace 无默认特性：796 项通过。
- 独立 core/eval/pipeline 无默认特性：165 项通过。
- Python 工具：64 项通过。

新回归覆盖 32 位容量、弱表回收、退休不复用、并发登记、耗尽、跨 Context 修订隔离、
数字 IR 逃逸、缺失名称拥有关系、显式导入、数字模块链接、COW 重定位、HTML 名称寿命、
宿主数字模块表及稳定排序、外来作用域、record 和 call/contract 身份不一致。
没有放宽模块绑定缓存的指针复用断言。

## 性能证据

独立证据目录：`/tmp/opencode/notist-definition-identity-complete-20261108`。
`tools/profile_definition_identity.py` 冻结候选与基线源码、校验输入和二进制，分别执行
clean release 和插桩分配账本；consumer 输入只读，仅在已归档站点副本重新发布 dist。

### 正式整站 A/B

两版本各预热一次，再执行八组交错完整构建，所有样本保留。

| clean 中位数 | 已修复 SourceId 基线 | 完整 DefId / ModuleId |
| --- | ---: | ---: |
| wall | 9.3569 s | 8.2931 s |
| user CPU | 8.5800 s | 7.6060 s |
| system CPU | 1.0616 s | 0.9573 s |
| 峰值 RSS | 3,321,230 KiB | 2,818,610 KiB |

配对 wall 比值中位数下降 **11.28%**，八组均更快；user CPU 中位数下降 **11.35%**，
RSS 中位数减少 **490.84 MiB**。这批绝对时间不能与旧局部版本在另一轮实验里的时间
直接拼接，也不能据此声称完整迁移比局部版本更快或更慢。全部 clean 样本均无 major
fault；这仍是当前机器、当前输入的观察，不是跨机器保证或新的 CPU 调用栈归因。

本机布局实测：DefId / ModuleId 各 4 B，FunctionId / ModuleRef 各 8 B，ImportBinding
与 ExtensionCtor 各 8 B；Node 104 B，Invocation 144 B，TypedExpr 136 B。拥有型
FunctionInstance 和裸引用型 ResolvedInstance 都为 32 B（包含 Vec 与对齐），不能
把 4 字节字段宣传成整个实例只有 4 字节。Sources 为 32 B，保留名称快照的成本计入测量。

### 独立 Rust 分配账本

两边分别构建同一 System allocator 计数探针的诊断副本。其耗时不用于 clean 加速比；
Rust 请求布局不包括 Python 扩展分配，realloc 字节指完整新布局，不等于实际复制字节。

| 精确计数 | SourceId 基线 | 完整迁移 |
| --- | ---: | ---: |
| 普通分配次数 | 181,337,444 | 134,854,678 |
| realloc 次数 | 14,776,801 | 14,829,053 |
| 普通分配累计请求字节 | 10,452,775,219 | 9,485,556,921 |
| realloc 累计请求字节 | 3,300,140,301 | 3,296,209,313 |
| requested-live 峰值 | 2,691,860,335 | 2,364,878,756 |
| CLI 局部析构后 requested-live | 2,953,498 | 5,120,510 |

普通分配减少 **46,482,766 次（25.63%）**，累计普通请求字节减少 **9.25%**，
requested-live 峰值减少 **311.83 MiB**。完整拥有关系并非零成本：realloc 增加
**52,252 次（0.35%）**，CLI 局部析构后的请求堆多 **2.07 MiB**；不能沿用局部版本的
“realloc 不增加、尾端仅多 855 字节”结论。这一尾端仍包含存活的进程缓存，不是纯身份表
大小，也不足以证明已做过长期 LSP 编辑压力测试。弱表键回收由单元测试独立验证。

RSS 与请求堆的统计口径不同，不把差额全部归因到某个字段或分配器碎片。

## 完整性与交付

预热、八组 clean 和两次 ledger 共 **20 次完整构建**，每次均在计时外核对
**78,004 项 / 451,812,631 字节**产物，全部一致。最终 verify 同时确认全部冻结源码、
四个运行二进制、原输入和完整输出清单没有变化。

证据目录保留 manifest、完整源码、实现补丁及改动清单、测试日志和汇总、clean 原始样本、
ledger、布局探针与运行二进制。源码冻结之后只新增本验收报告，没有再修改候选生产代码。
P2 的内部身份迁移与本轮验收完成；代码和文档保持未提交、未推送，不自动开始下一项 ID。

复现须使用新的外部目录，不能覆盖本轮或旧局部版本证据：

```sh
python3 tools/profile_definition_identity.py prepare --out /tmp/opencode/definition-complete-REPRO
python3 tools/profile_definition_identity.py build --out /tmp/opencode/definition-complete-REPRO
python3 tools/profile_definition_identity.py measure --out /tmp/opencode/definition-complete-REPRO
python3 tools/profile_definition_identity.py ledgers --out /tmp/opencode/definition-complete-REPRO
python3 tools/profile_definition_identity.py verify --out /tmp/opencode/definition-complete-REPRO
```
