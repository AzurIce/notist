# BindingId / TypeParamId 完整重构验收

## 状态与证据边界

实现、完整性审计、完整测试、整站 clean / ledger 测量与最终校验均已完成，P3 在本报告
范围内完成。代码未提交、未推送，不自动开始下一项。

新证据目录：`/tmp/opencode/notist-binding-identity-complete-20261108-v2`。
基线来自实际 HEAD `56116bc98080d8dfd8fd5b940c5e8ab5be8b1789` 的 git archive；
候选来自当前工作区冻结。该 HEAD 与先前记录的 f189b33 只差 LICENSE，仍重新构建，
没有拿上一轮可执行文件代替本轮基线。

重要纠错：历史部分候选的 clean before / after SHA-256 相同，共享 Cargo target
误复用了基线。原报告中 wall −0.46%、user CPU −0.81% 及 clean RSS 对比均作废，
clean 输出检查也不能验收当时的候选。原目录与原始样本保留，历史报告开头已纠正；
旧 ledger 即使哈希不同也只是部分候选，不能替代本报告。

首次完整冻结在计时前发现相同问题并被否决，保留为无效实验。本轮工具对 before、
after、before-ledger、after-ledger 分别隔离 Cargo target，并拒绝正常版本哈希相同的
构建；新增 Python 回归检查隔离和拒绝逻辑。不把构建错误造成的差异算作性能收益。

## 实现完整性

- BindingId / TypeParamId 是独立领域的 32 位非零数字，Option 也是 4 字节，共用私有
  泛型表示与单调分配机制；数字不序列化、不回收复用，耗尽明确失败。
- 已解析 Input、Block、For、ResultPattern 与 InstanceRecord.inputs 使用 BindingId。
  求值 Scope 是数字键的 COW 映射，捕获内容仍共享代码和当时环境，不用替换共享代码
  中的值模拟捕获。保留先求值实参、再取得特化实现的顺序与原预算。
- 检查器的名字表只负责解析源码；调用参数名与字段名仍属于边界协议，不混入 Symbol、
  FieldId、InstanceId、结构化特化键、native 协议改写或 Scope 槽位化。
- ResolvedSignature 按声明顺序直接保留 resolved 编译期参数、运行时参数、返回值和
  children 模板，不再使用 BTreeMap<Type, ResolvedType> 侧表。字段只读，签名保留权威
  声明快照；Registry 共享原 DefinitionModule，HTML 声明与签名共享 FunctionDef Arc。
- ComptimeBindings 按 ComptimeId 读写，infer / concrete / require 直接消费 ResolvedType。
  未绑定返回显式 None，要求具体类型时出错；外来 ID、具名模板混入 resolved 图、
  非具体 Type 实参和默认值均拒绝，不再静默复制旧模板或跳过推断。移除 Type.substitute。
- 检查器、合成调用、HTML 参数检查与 evaluator API 示例均使用该 resolved 通路。
  extension 合约按拥有签名内的参数声明位置比较 alpha-equivalence，不按拼写归一化；
  宿主内容结果筛选也消费已解析返回模板。

## 拥有关系、wire 与缓存

Bindings 随 Sources 持有名称与声明元数据。源码域只有弱声明地址索引，地址包含定义函数、
名称、声明与作用域位置；强拥有者保留可选 FunctionId，因此同一物理源码被不同模块或
函数解释时不共用绑定。文档根声明没有函数拥有者。重复分析、多个特化仍共享活着的同一
声明；最后拥有者释放时按指针移除弱键，不永久保留历次编辑的局部名字。

局部 IR wire 编码 name / source / span / scope / 可选 owner 描述。owner 是模块名与
函数名，不是 DefId。严格解码只匹配快照中已有的完整描述，不跨来源、范围或函数拥有者
回退；只有显式 import 可为新声明建立拥有关系。结构环境检查遮蔽，动态自由引用要求
decode_with_bindings 显式环境，不保留裸名字兼容分支、unique(name) 或区间猜测。
常量别名使用相同的显式描述，不为未知动态自由变量伪造运行时值。

可缓存程序先投影掉已丢弃调用者来源的局部声明元数据，再验证实际引用。成功 ModuleScope
结果增加弱规范化，修复 comemo 并发计算可能产生等值但不共享指针的结果：键只含名称
拥有关系与导入导出结果，不含实现、签名或来源修订；失败与禁用缓存路径不复用。最后
一个结果释放时按指针删除键；保留原指针共享断言，不靠重试掩盖该问题。

## 完整测试

- workspace all-features：842 项通过。
- workspace no-default-features：826 项通过，串行执行测试。
- 独立 core / eval / pipeline no-default-features：187 项通过，串行执行测试。
- Python 工具：67 项通过。
- 模块共享测试套件默认并发执行，连续 20 轮通过；另有并发规范化与弱键回收专门测试。
- 冻结候选上的独立 profiling 探针验证：启用缓存共享结果，禁用后不复用池中结果且两次
  调用也不共用指针，重新启用恢复原共享结果。探针源码与日志随证据保留。
- cargo check workspace all-features、rustfmt 与 git diff --check 通过。

一次并发 no-default 套件在既有 CLI 预览 socket 测试中出现 ConnectionRefused /
ConnectionReset。失败日志单独保留，随后完整套件串行执行通过；未改预览代码或放宽断言，
不将该失败隐去。模块共享的旧竞态则是实现修复后通过，不与预览测试失败混为一谈。

新增／加强回归覆盖：32 位与超过 16 位容量、弱键回收和退休不复用、COW 名称快照、
嵌套循环和 Result 载荷遮蔽、显式自由环境与跨来源／作用域拒绝、同源码位置不同函数拥有者、
拥有者随逃逸快照保留、同源码编译到两个模块、签名权威声明、无侧表结构模板、外来参数 ID、
非具体 Type 值／默认值、Never 与冲突推断、extension 参数改名／重排、HTML 更新签名、
缓存去除调用者局部元数据。已有捕获、错误顺序、预算、磁盘／worker 一致性断言未放宽。

## 整站性能与最终校验

输入复用已验收只读工作负载与完整输出清单，不改 consumer、Cargo.lock 或默认预算；
未重启 CPU 采样服务器。构建与输出核对不计入 clean 时间，插桩耗时不混入 clean 加速比。

### 八组 clean A/B

各版本预热一次，再做八组交错完整构建，保留全部样本。正常基线 SHA-256 为
079cf9126ee948755858aff45a55b17237b22a7942cbc7186b8f374bf5ddb8f6，候选为
b1391d951a19aa7672b9dd7e15bdec6f43169224580f456099466c2cc69923b3；两者独立从对应
冻结源码构建，正常构建耗时各约 1m43s。它们不是历史错误实验中的同一二进制。

| clean 中位数 | 已提交 DefId 基线 | 完整 BindingId 候选 |
| --- | ---: | ---: |
| wall | 8.2471 s | 8.1230 s |
| user CPU | 7.5404 s | 7.4147 s |
| system CPU | 0.9599 s | 0.9715 s |
| 峰值 RSS | 2,816,138 KiB | 2,824,960 KiB |

配对 wall 比值中位数 −1.51%，八组均更快；user CPU 中位数下降 1.67%。收益较小，
不是与上轮不同环境的绝对秒数拼接。代价是 system CPU 中位数约增加 1.22%，RSS
中位数增加 8,822 KiB（8.62 MiB）；不声称内存峰值改善。

两次预热与十六次 clean 全部完成，每次在计时外核对完整 78,004 项／451,812,631 字节
输出清单，均一致。

### 独立分配账本

两个版本各自从冻结源码复制并插入相同的 System allocator 探针，分别使用独立 target
构建。普通分配含 alloc / alloc_zeroed；累计请求字节不是 RSS，realloc 计完整新布局，
不代表实际复制量；探针不包含 Python 扩展的分配。不把这两次插桩运行耗时混入 clean
加速比，也不依据单次插桩 CPU 时间推断性能收益。

| 精确计数 | 已提交 DefId 基线 | 完整 BindingId 候选 |
| --- | ---: | ---: |
| 普通分配次数 | 134,854,678 | 132,729,763 |
| realloc 次数 | 14,829,053 | 14,829,064 |
| 普通分配累计请求字节 | 9,485,556,925 | 9,430,193,110 |
| realloc 累计请求字节 | 3,296,209,317 | 3,299,355,505 |
| requested-live 峰值 | 2,364,878,756 | 2,369,167,666 |
| CLI 局部析构后 requested-live | 5,120,510 | 5,136,879 |

普通分配减少 2,124,915 次（1.58%），普通累计请求减少 55,363,815 字节（0.58%）。
代价是 realloc 多 11 次，累计 realloc 请求多 3,146,188 字节（约 3.00 MiB），
请求堆峰值增加 4,288,910 字节（4.09 MiB），CLI 局部析构后增加 16,369 字节。
RSS 与请求堆峰值都没有改善；本轮交付是完整的身份与权威表示收敛，附带较小的 CPU /
wall 与分配收益，不是内存峰值优化或长期 LSP 编辑压力测试结论。

### 最终完整性与交付

两次预热、十六次 clean 与两次独立 ledger 共 20 次完整构建，每次均核对完整
78,004 项／451,812,631 字节输出，全部一致。最终 verify 核对两份正常源码、两份探针
源码、四个二进制、冻结输入与完整输出清单，全部通过。四个二进制哈希互不相同。

证据目录保留 manifest、冻结源码、四个二进制、工作区补丁和 untracked 清单、测试日志与
汇总、并发预览失败原始日志、20 轮模块共享测试日志、禁用缓存探针、完整性审计、clean
原始样本和汇总、ledger 原始账本与汇总。候选冻结后未修改实现或测试；仅新增本报告及
补充历史报告的错误证据纠正，历史无效目录未覆盖。

复现必须使用新的外部证据目录与新的 target 根目录：

```sh
python3 tools/profile_binding_identity.py prepare --out /tmp/opencode/binding-REPRO --target /tmp/opencode/binding-target-REPRO
python3 tools/profile_binding_identity.py build --out /tmp/opencode/binding-REPRO --target /tmp/opencode/binding-target-REPRO
python3 tools/profile_binding_identity.py measure --out /tmp/opencode/binding-REPRO --target /tmp/opencode/binding-target-REPRO
python3 tools/profile_binding_identity.py ledgers --out /tmp/opencode/binding-REPRO --target /tmp/opencode/binding-target-REPRO
python3 tools/profile_binding_identity.py verify --out /tmp/opencode/binding-REPRO --target /tmp/opencode/binding-target-REPRO
```
