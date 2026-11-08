# BindingId / TypeParamId：局部引用与已解析签名

## 构建证据补充纠错：clean 性能结论作废

重新核对 manifest 后发现，本目录所述历史证据的 before / after clean 可执行文件
SHA-256 完全相同：996282d446d7e099f7b5bff8bd1d4bc5163e29be4067725d98346e406555768e。
共享 Cargo target 与复制源码的旧 mtime 使候选构建复用了基线产物。因此下文 clean
时间、CPU、RSS 只能视为同一程序的重复测量，不能归因于 BindingId 迁移；对应 clean
输出验证也不构成候选输出的验收。历史原始证据保留，不覆盖，不再引用其提速结论。
ledger 两个哈希不同，但它们仍只是旧的部分候选，不能替代完整候选验收。

新的完整验收必须隔离每个正常／插桩版本的 Cargo target，拒绝两个正常版本哈希相同的
实验，并重新完成测量与输出核对。首次完整冻结目录同样在计时前被该检查否决，未使用。

## 完整性复核：尚非完整、干净的内部重构

用户追问完整性后重新检查，发现此前将 P3 标记为完整过早。下文测试和性能数字仍然
对应已冻结的当前实现，但只能验收该候选，不能证明所有内部身份边界已经收敛。

- ResolvedSignature 仍以 `BTreeMap<Type, ResolvedType>` 作为侧表。内部推断／替换入口
  仍接收旧具名 Type 并回查模板，没有直接消费按声明组织的 resolved 参数／返回值／
  children 类型；ComptimeBindings 的主要读写入口也仍从参数名查数字键。
- `ComptimeBindings.substitute` 在模板未登记时直接返回原 Type；infer 在这种情况下
  返回空错误列表、不提供推断。最小复现已确认：未索引的 Option<T> 模板被静默略过。
  这不是对当前普通调用已发现错误的断言，但它会掩盖遗漏的内部迁移入口。
- `Bindings.reference` 在没有匹配来源和作用域的声明时调用 `unique(name)`。最小
  复现已确认：另一个来源中的引用可取得快照里唯一的同名声明。运行时 Scope 仍按
  数字键且没有名字取值回退，但 wire 的词法恢复边界并不严格，不能称为干净收尾。

需要将已解析签名改为内部权威表示，区分显式未绑定状态与缺失模板错误，并收紧 wire
自由变量的显式上下文和作用域检查，再重新验证。P3 已重新打开，代码仍未提交。
复现程序：`/tmp/opencode/binding-identity-audit.rs`。

## 范围

以已提交的 `f189b33` 为基线，迁移局部参数、let、for、Result 模式的已解析引用和求值
环境，并为签名中的类型参数建立数字身份。不改 InstanceId、字段 Symbol、命名调用协议、
默认预算或宿主输出生命周期，不把 Scope 同时改成槽位数组。

## 内部通路与拥有关系

- BindingId / TypeParamId 都是 32 位非零数字，Option 也为 4 字节，共用私有泛型底层。
  编号耗尽不回绕；数字不直接序列化。
- ExprKind::Input、Block 声明、For 绑定与 ResultPattern 使用 BindingId。求值 Scope
  的 COW BTreeMap 改为数字键；保存内容仍共享代码与当时的环境，不替换共享代码中的值。
- InstanceRecord.inputs 保留参数名到 BindingId 的调用边界投影，函数体不按拼写回退。
  仍先求值实参再取得特化实现，不为了先取得参数 ID 提前报告特化失败。
- Registry 安装具名声明时生成 ResolvedSignature；内部 ResolvedType::Parameter 保存
  TypeParamId，ComptimeBindings 保存数字参数键。递归推断、替换、合成调用和 HTML
  组件参数检查使用这条通路。具名 Type::Parameter(String) 和公开 Type.substitute
  保留为外部声明接口，不声称已经移除全部文字输入或诊断投影。
- 编译期常量折叠、已有 Option 模式的表达式替换及分支裁剪保持原规则；已消去的运行时
  绑定不为了身份化而重新执行。编译期实参不再额外塞进无引用的运行时字符串 Scope。

初次回归发现：如果每次检查都重新分配局部数字，同一源码的两次分析及重复求值结果
不再相等。没有放宽原有 Analysis / Evaluation 相等性测试，而是修正了身份寿命边界：

- 源码域拥有弱的声明地址索引，地址包括声明名、声明位置和词法作用域位置。
- Sources 携带的 Bindings 持有强拥有者；活着的同一源码声明共享身份。遮蔽声明、不同
  来源域、不同位置的同名变量不合并；身份不是实现修订或跨 Context 的值缓存许可。
- 最后一个声明拥有者释放时核对指针并删除弱键。源码仍存活不会永久保留历次任意
  局部名字。索引在源码域内，不是全局局部名字 interner；全局只保留数字计数器，方便
  合并独立程序和特化而不重写 IR。

## Wire 与校验

wire 保持局部名字的文字表示，通过 let／for／match 结构恢复词法环境。抽取函数体时
可用 Sources.decode_with_bindings 显式提供 InstanceRecord.inputs，未知或含糊的动态
自由变量报错。常量别名可随显式 import 保留显示名；不会凭名字构造运行时值。

名称快照保留原声明和作用域范围，因此编译期分支裁剪缩窄表达式范围后仍能解释其
所属声明。缺失绑定拥有关系、错误的参数名投影或遗漏声明来源均会被验证拒绝。SourceId
校验的 Item／Expr 互斥遍历保持不变。

## 正确性验收

- workspace 全特性：830 项通过。
- workspace 无默认特性：814 项通过。
- 独立 core/eval/pipeline 无默认特性：179 项通过。
- Python 工具：66 项通过。

新增回归覆盖 32 位／超过 16 位容量、同源码重复声明身份、弱键回收、退休不复用、
COW 名称快照、嵌套循环遮蔽与外层恢复、同名外来 ID 不按名字取得参数、Result 分支
载荷遮蔽和裁剪后 wire、显式自由变量上下文、独立类型参数及冲突／Never 推断规则。
已有捕获、循环预算、替换、实际组件、磁盘与 worker 一致性断言没有放宽。

## 完整整站验收

证据目录：`/tmp/opencode/notist-binding-identity-20261108`。
`tools/profile_binding_identity.py` 冻结候选源码，并从 git archive 导出的 `f189b33`
重新构建无插桩基线；不把更早 DefId 候选的二进制冒充本轮起点。复用已验收的只读输入
与完整产物清单，clean 与分配账本独立构建和运行。

### 八组 clean A/B

各版本预热一次，再做八组交错完整构建，全部样本保留。

| clean 中位数 | 已提交 DefId 基线 | BindingId / TypeParamId |
| --- | ---: | ---: |
| wall | 8.2351 s | 8.1977 s |
| user CPU | 7.5457 s | 7.4848 s |
| system CPU | 0.9707 s | 0.9630 s |
| 峰值 RSS | 2,817,976 KiB | 2,817,130 KiB |

配对 wall 比值中位数 **−0.46%**，八组中七组更快，一组约慢 0.05%；user CPU
中位数 **−0.81%**。RSS 中位数仅减少 **0.83 MiB**，不把这种小差异宣传为显著内存收益。
没有重启 CPU 采样服务器，也不拿这批绝对时间与上轮不同实验环境的秒数拼接提速比。

本机布局验证：BindingId、TypeParamId 及其 Option 均为 4 B；Sources 为 40 B，
InstanceRecord 为 312 B；ResultPattern 为 8 B，ResultArm 为 144 B。Node 104 B、
Invocation 144 B、TypedExpr 136 B，未因局部引用变小而进一步缩小。这些是当前机器的
实测布局，不是稳定 ABI 承诺，也不能把 4 字节 ID 字段等同于整个表达式大小。

### 独立分配账本

两个版本分别构建同一 System allocator 探针的诊断副本，不把插桩耗时混入 clean
加速比。普通分配含 alloc / alloc_zeroed；累计请求字节不是实际驻留内存，realloc
字节计完整新布局，不代表实际复制量；计数不包括 Python 扩展分配。

| 精确计数 | 已提交 DefId 基线 | 本轮 |
| --- | ---: | ---: |
| 普通分配次数 | 134,854,678 | 132,729,895 |
| realloc 次数 | 14,829,053 | 14,829,057 |
| 普通分配累计请求字节 | 9,485,556,913 | 9,429,780,416 |
| realloc 累计请求字节 | 3,296,209,317 | 3,299,352,865 |
| requested-live 峰值 | 2,364,878,756 | 2,368,765,673 |
| CLI 局部析构后 requested-live | 5,120,510 | 5,160,086 |

普通分配减少 **2,124,783 次（1.58%）**，累计普通请求字节减少 **0.59%**。新增声明
拥有关系与已解析签名也有成本：realloc 多 **4 次**，累计 realloc 请求字节增加
约 **3.00 MiB**，请求堆峰值增加 **3.71 MiB**，CLI 局部析构后多 **39,576 字节**。
不能因为 RSS 中位数略低就声称本轮降低了请求堆峰值；两种统计口径不同。

本轮的主要交付是身份与解析边界完整、减少名字分配；整站 CPU / wall 收益较小，
内存峰值并未改善。弱键回收由独立测试验证，尚未据此宣称完成长期 LSP 编辑压力测试。

## 最终完整性与交付

预热、八组 clean 与两次 ledger 共 **20 次完整构建**，每次均在计时外核对全部
**78,004 项 / 451,812,631 字节**产物，全部一致。最终 verify 校验冻结的两份正常源码、
两份诊断源码、四个运行二进制、原始输入以及完整输出清单，全部通过。

证据目录保留 manifest、源码与二进制、实现补丁、完整测试日志及汇总、原始 clean
样本、独立账本和布局探针。候选冻结后只补写本报告，没有修改候选生产代码。
以上是当前候选已有实现的测试和性能验收，不等同于 P3 的完整性完成；见开头复核。
代码与文档保持未提交、未推送，不自动开始 InstanceId、Symbol 或 native 调用协议改造。

复现使用新的外部目录，避免覆盖已有证据：

```sh
python3 tools/profile_binding_identity.py prepare --out /tmp/opencode/binding-identity-REPRO
python3 tools/profile_binding_identity.py build --out /tmp/opencode/binding-identity-REPRO
python3 tools/profile_binding_identity.py measure --out /tmp/opencode/binding-identity-REPRO
python3 tools/profile_binding_identity.py ledgers --out /tmp/opencode/binding-identity-REPRO
python3 tools/profile_binding_identity.py verify --out /tmp/opencode/binding-identity-REPRO
```

prepare 以执行时的 HEAD 为基线，因此复现这次历史对照时应先确认 HEAD 为 `f189b33`，
候选改动在工作区；不能在提交候选后直接运行而把候选自身当成基线。
