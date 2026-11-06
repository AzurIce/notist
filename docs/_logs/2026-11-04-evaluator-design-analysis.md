# Notist 求值器深度分析：与 Haskell/GHC、Typst 的对照

## 结论先行

**建议保留 Notist 的“严格数据 + 延迟、逐挂载执行的内容”语义，优先把运行时从“复制并代换程序树”改进为“共享程序、显式捕获环境、低成本实例调用”。不建议把整门语言改成 Haskell 式 call-by-need，也不建议先全面 memoize 求值或重写字节码 VM。**

值得借鉴的是：

1. **GHC 的闭包转换、共享不可变程序、已知调用的专门化**，不是简单的“所有东西都加 thunk”。
2. **Typst 的内容句柄/COW、代码与捕获值分离、带实际依赖的缓存边界**，不是误以为 Typst 靠全局惰性求值或字节码获得性能。
3. **明确区分共享代码、共享内容配方、共享展开结果**。前两项适合 Notist；第三项会碰到挂载身份、资源请求、预算、transform 和错误来源，不能默认等同。

本轮仅分析，没有改求值器实现。数值依据前一轮全量 profile；另增加了类型布局测量和四个语义探针，没有对 Haskell/Typst 做吞吐基准，不给出跨语言性能排名。

## 1. 证据与版本边界

- Notist 性能基线：`85ab69e13bb99cb4016538accdd55c65f0cb6d7c`，完整 IceXIV 51,739 页。
- 当前工作区的 evaluator / core 相关文件与该轮分析对照；进行中的 CLI/progress 等改动不属于本轮实现变更。
- Typst 源码：`84cca406aafe7f830ed3f37f1cc6f0c8b6b65a03`，2026-10-28；固定源码副本在 `/tmp/opencode/notist-evaluator-analysis-20261104/typst`。
- GHC：运行时对照 `ghc-9.12.2-release` 的 `rts/Updates.h`，优化机制参考 GHC 官方优化文档。
- 上轮指标与完整性见 [IceXIV profile](2026-11-04-icexiv-profile.md)。本轮探针源码和 stdout 在 `/tmp/opencode/notist-evaluator-analysis-20261104/`。

应严格区分三类结论：**实测事实**、**源码可确认的操作/语义**、**尚待 A/B 验证的改进假设**。例如 6.54 GiB RSS 不能直接分配给某种节点；某条 `.clone()` 路径存在，也不证明它贡献了确定的多少秒。

## 2. Notist 当前究竟是什么求值模型

### 2.1 不是整体严格，也不是整体惰性

`crates/notist-pipeline/src/code/eval.rs` 中：

| 位置 | 语义 |
| --- | --- |
| `expr::Block`，308–315 | 顺序、严格地求值每个 let 初始化式，然后求值结果 |
| `expr::If`，316–319 | 求值条件，仅执行被选中的分支 |
| `expr::Binary`，324–335 | 普通参数严格，`&&` / `||` 短路 |
| `expr::ValueCall`，395–403 | 执行数据调用，子表达式按实际路径求值 |
| `expr::Content`，280–290 | 绑定内容，但不执行其中 opaque 内容调用 |
| `expr::For`，292–307 | 捕获一个延迟 Repeat，挂载时才执行 subject/body |
| `present_call`，616–664 | 挂载内容调用时执行实现，再展开返回内容 |

因此：`let value = text(text: panic("unused")); [ok]` 不触发 panic，但不代表任意 let 都惰性。这个 `text(...)` 是返回内容的调用，被 checker 降为待展开的 Item，而不是立即执行的数据调用。

静态检查仍可能拒绝未选中的非法结构。**运行时不执行分支，不等于 checker 可以忽略分支**；表格 row/cell 等结构角色已有相关测试。

### 2.2 最重要的性质：延迟内容是可重放配方

用冻结 profile 库直接运行四个独立小探针，输出如下：

```text
unused_content: reads=0, diagnostics=0
two_mounts: reads=2, diagnostics=0
strict_outer_value: reads=1, diagnostics=0
unused_content_argument: reads=0, diagnostics=0
```

对应核心程序，省略 Data decode 的样板：

```text
let v = for x in read_values() [#text(text: str(x))];
[ok]                 // 不读取

let v = for x in read_values() [#text(text: str(x))];
[#v#v]               // 每次挂载执行 subject：逻辑读取两次

let xs = read_values();
let v = for x in xs [#text(text: str(x))];
[#v#v]               // 外层严格 let：逻辑读取一次
```

这不是推测：`tests/data_bodies.rs:283–339` 也明确断言两次挂载产生两次读取，以及外层读取保持严格。物理文件字节/JSON 树仍可命中宿主缓存；**逻辑执行两次与物理解析两次是两回事**。

更接近的概念是：一个可重放的 document computation / recipe，而不是“求值一次并把结果填回共享格子”的默认 memo thunk。与 Haskell 的类比，复用一个纯值与复用一个 IO/Reader/State 风格的动作描述不是同一件事：动作描述共享，不自动意味着动作结果共享。

### 2.3 “纯”必须说明包含哪些参数

固定输入视图下，求值可以确定性地建模为函数；但不能把它简化为只有 AST 的函数：

```text
Eval(Item/recipe, Context, Inputs, Limits, mount state)
  -> tree + diagnostics + dependencies + needed resources
```

`read_json` / `read_text` 使用注入的输入能力，而非任意环境 IO；仍会产生逻辑请求、依赖、Pending 和配额影响。`present_call` 每个挂载重置局部 steps，而 iterations / loop_work 等又在更大范围累计。缓存或融合若只保留最终文本，会丢掉这些可观察行为。

### 2.4 也不是直接实现 Haskell 的 lambda calculus

当前 Runtime、Value、Type 中没有通用的函数值/箭头类型；函数主要以名义 FunctionId 和具体 FunctionInstance 调用。内容是一等值，并不等于已有任意高阶函数、Hindley–Milner 推断或 Haskell 类型类模型。

显式 comptime 参数和 `TypedExpr.known` 又形成了一个 staging/部分求值维度：**编译时可算、运行时严格、内容挂载时才算，是三个不同问题**。建议的内部闭包式内容表示不要求给用户语言新增 lambda/一等函数。

解释器还主动限制物理调用深度至 32，并维护操作/迭代预算。这与 GHC 原生代码的尾调用/循环优化是不同契约；不能仅为了消除解释器栈就顺便放开递归限制或改变预算失败位置。

## 3. 现在的执行路径：为什么会花这么多时间

### 3.1 两种运行时值

```rust
enum Runtime {
    Value(ComptimeValue),
    Content(Type, Arc<Vec<Item>>),
}
type Scope = BTreeMap<String, Runtime>;
```

`run` 先对输入根执行 `bind_content`，再执行 `present`。`CheckedProgram` 已经是 `Arc<BTreeMap<String, Arc<InstanceRecord>>>`；`program.clone()` **不是复制全部检查结果**。Text、Array、Dict 也已有共享容器和部分缓存，不能把当前实现描述成完全没有共享。

但共享检查结果和数据容器，并没有消除**运行时内容树、调用信息和捕获表达式的复制**。

### 3.2 一次 `views.item(...)` 的概念路径

1. 页面里的 Pending Call 绑定参数，暂不展开实现。
2. `present_call` 确认实际 transform target、挂载预算和恢复边界。
3. `execute` 求值参数；构造字段 Dict，Code 实现还要建立 Scope。
4. `implementation` 构造实例 key，查询已检查实例；缺失才进入 checker。
5. 执行函数 Block，严格完成 read / Data 访问 / let。
6. 函数返回内容：`bind_content` 遍历 Item，`capture_call/capture_expr` 将自由输入代换进待执行表达式。
7. 保留调用边界，将返回 forest 放入其 children；附加 expansion 来源。
8. 再 `present` 返回 forest，执行其中的内容构造调用，直到没有 pending。

这不是一个“单次解释 AST，直接产出最终节点”的过程。它在**解释表达式、把代码绑定成内容、代换捕获、展开内容、保留边界**之间往返。

### 3.3 profile 能确认什么

原顺序 coarse 对照的全站求值均值约 **7.216s**；SSG + Vault 收尾约 **3.090s**。密集 detail 则记录：

| 函数 | 调用数 | detail Self |
| --- | ---: | ---: |
| `present` | 8,492,164 | 3.608s |
| `execute` | 6,073,286 | 2.902s |
| `bind_content` | 4,426,913 | 1.870s |
| `implementation` | 6,073,286 | 0.894s |

**调用数不是不同输出节点数。** 例如 `present_node` 给叶子也调用 `present(empty_children)`；`present` 统计的是 forest 处理函数的进入次数，还包括挂载返回的额外边界。不能据 849 万次调用推导出 849 万个同时存活的 Item。

detail 比正常版整体慢约 17.6%，高频探针还会影响内联及成本归属；不能断言去掉某个函数就能省它的全部 Self。可靠结论是该区域值得优化，具体收益仍要用无插桩 A/B 证明。

## 4. 六个具体实现问题

### A. 捕获用的是递归代换，不是“共享代码 + 环境”

`capture_expr`，1133–1229：

- 输入是 data 时，建立带 known 值的新 TypedExpr。
- 输入是 content 时，直接 `ExprKind::Content(forest.as_ref().clone())`。
- 遍历数组、字典、条件两支、调用参数、嵌套内容、Block、for、match 等。
- 为维持遮蔽，复制 Scope 并 remove 局部绑定。

它不执行未选中的分支，这点是正确的；但**不执行仍可能遍历并复制那一支**。一般而言，捕获一次的成本随表达式/内容子树大小增长，而非仅随自由变量数量增长。

`capture_call` 还 clone `Box<FunctionDef>`。FunctionDef 的参数列表、名称等不是一个廉价共享引用。共享函数体的好处会在构造每次调用的捕获副本时被部分抵消。

**值得验证的替代**：共享 checked body/Item 模板，计算自由变量集合；运行时建立小的 captured values 环境，而非递归重写代码。它是 closure conversion 的思路，**不要求引入 call-by-need 或结果缓存**。

### B. `Arc<Vec<Item>>` 没让递归树成为廉价值

`Item.children` 仍是 `Vec<Item>`，Item 使用 derive Clone。复制 Vec<Item> 会复制 Item 并递归复制其 children 和 owned pending 等。

`Arc::unwrap_or_clone` 有良好的 unique fast path；**不是每次调用都深拷贝**。但内容仍被 Scope 或其它挂载持有时，消费一份 clone 就需要复制整个 Vec 树。`Arc::make_mut` 在 origin expansion 等修改前也可能触发 COW。

因此当前模型更接近“根 forest 被共享、需要独立修改时物化拥有树”，并非 Typst 风格的逐层共享内容句柄。

**值得验证的替代**：共享不可变结构/配方，单独持有每次 mount 的身份和来源；或先共享高频的签名、源文件标识和子树容器。不能只给现有拥有树多套一层 Arc，然后继续逐后代写入 origin，期望自动解决复制。

### C. 已知的调用，仍走很通用的运行时通路

`execute`，810–1053：

- 每次为实际 target 复制 FunctionInstance 和 comptime arguments。
- 查询 registry 判断 Code/native。
- 对每个参数按名字在线性参数列表里 `find`。
- 将 data 参数写入 fields；Code 调用另外写入 inputs Scope。
- 再进入 `implementation`，按目标和 comptime 参数格式化字符串 key。
- 再分派到 native / Code；native 内部再从 fields 按字符串取值。

对于非常小的 `data_expect`、`data_require_index`、`str`、`text`，调用框架可比真正操作更重。IceXIV 的 Data native 约 196 万次，实际 JSON parse 只有 565 次；把精力全放在 JSON parser 方向不对。

`FunctionInstance::key`（`definitions.rs:279–288`）明确构造 String，并收集各 comptime 值的 key。`implementation` 命中时还做 `contains_key` 后 `get`。改成一次 lookup 是局部改善，但更重要的是减少数百万次重复构造语义 key。

**值得验证的替代**：已验证的调用记录中保存参数布局和 Context-local 实例 ID；原生函数接受 slot 参数切片，必要时才形成可观察的 fields。对外仍保留 FunctionInstance 符号身份和原有 wire 形式。

注意 `panic` 或 `text` 可能被用户定义/transform 替换；快路径必须依据**实际解析到的实现身份**，不能按源码名称硬编码。

### D. metadata 和恢复边界在成功路径上也付费

`present` 每个 node 先 `item_shell` 复制恢复边界，即使最后不会失败。fields/attrs 的共享副本可能较廉价，但 instance 的 String、origin 的 source/expansion 等仍有复制成本。

Code 内容调用返回时，`expand` 又递归给后代添加 expansion。嵌套展开会重复接触后代，成本可随输出大小与调用嵌套深度共同增长。

**值得验证的替代**：小而共享的恢复 checkpoint、SourceId、持久化来源链或挂载来源帧，在需要诊断/序列化/源码映射时投影。不是丢弃来源信息；相对路径解析、错误内层位置、外层调用栈仍必须正确。

### E. 作用域没有利用已知的词法结构

Block、match、capture 等复制 BTreeMap；名字是 String。深度通常不大，但在大量小调用下仍会重复构建容器。

**最小演进**：进入/离开 scope 的帧栈或带 parent 的环境，避免复制整个可见 Scope。

**更进一步**：checker 已知的局部名转为 frame slot，捕获环境只保存自由变量。需要保留名称用于诊断，并处理 for / match / Block 的遮蔽，不能以一个全局 name→slot 映射替代词法绑定。

### F. 统一 Item 承担了很大的固定成本

用当前 profile 冻结的 x86_64 Rust 库做 `size_of` 实测：

| 类型 | 不含外部分配的大小 |
| --- | ---: |
| `Item` | **344 B** |
| `Ctor` | 56 B |
| `Value` | 48 B |
| `Origin` | 56 B |
| `TypedExpr` | **160 B** |
| `Invocation` | **208 B** |
| `PendingEvaluation` | 208 B |
| `FunctionDef` | 160 B |
| `FunctionInstance` | 72 B |
| `FunctionId` | 48 B |

FunctionId 本身是两个 String；Item 同时可容纳 ctor、pending 指针、instance、target、evaluation、origin、fields、children、attrs、span，即使是一个完成后的文本叶子也承担这套布局。举例：100 万个 Item 仅内联结构就约 328 MiB，尚无字符串、Vec 容量、Box 和参数列表。

这说明 compact representation 有研究价值，但**不能把前述函数调用次数乘 344B 来推导 RSS**，也不能把所有 6.54 GiB 都归给 Item。这些尺寸不是稳定 Rust ABI，也不包含堆中的递归负载。

## 5. Haskell/GHC：应该学什么，不该学什么

### 5.1 call-by-need 与 Notist 的差别

典型 Haskell 共享绑定：

```haskell
let v = expensiveTree input
in (render v, inspect v)
```

当两个消费者指向同一可更新 thunk，GHC 通常对需要的部分求值，再更新该 thunk 指向结果，让后续消费共享。它不会因第二次引用就例行深克隆整棵纯树。也不会自动把所有独立写出的 `f x` 都变成一个跨参数全局缓存。

GHC `rts/Updates.h` 直接说明生命周期：thunk 携带 free variables → 被进入/可能 blackhole → 更新指向结果。具体实现并非朴素的“总是替换成 IND 对象”：该版本也利用 BLACKHOLE 形态表达结果间接引用。

Haskell 的非严格语义不承诺每个源表达式在所有实现和并发情形下物理上恰好执行一次；这里描述的是共享 thunk 的常见实现机制，不是可观察 IO 次数保证。

Notist 的同一内容两次挂载明确要求重复逻辑读取、独立挂载边界并累计预算。**把 Pending 内容原地 memoize 成 Expanded 树会改变现有测试的语义**。可以借鉴闭包表示，不能顺手借入 thunk update 的结果共享规则。

### 5.2 更值得借的是 closure conversion

GHC 闭包的核心抽象是 entry code + free-variable payload。代码本身可共享，环境携带需要的值，进入闭包时不需要先把变量递归替换进代码树。

对应 Notist 的建议概念：

```text
Content recipe = checked Item/body reference + captured values
Mount          = recipe + origin/attrs/transform context + budget state
```

多个 mount 共享同一 recipe，但各自执行，不共享其执行状态。这保持 Notist 的现有语义，同时移除递归捕获代换的一部分结构成本。

### 5.3 GHC 的快不只是“更懒”

GHC 常规编译路径把 Core / STG 等逐步降到原生代码，不是以 Rust BTreeMap 和 String 运行用户函数体。需求分析、worker/wrapper、专门化、内联和消除中间结构共同移除惰性抽象的开销。

对 Notist 最有启发的对应是：

- 已知目标和参数类型时，生成/保存直接调用布局，不每次重新构造通用协议。
- 区分普通公开 wrapper 与经过验证的内部 worker 路径。
- 消除 bind/capture/present 之间**不具有可观察身份**的中间表示；保留有注解、来源或恢复语义的边界。
- 数据预算、合法性检查是否能免除，需要证据和证书，不是删除 `accepts` 就叫静态优化。

另一个区别是求值深度：Haskell 消费者可能只需求值到 WHNF，树的后代继续保持延迟；Notist `present` 则需要完成实际挂载输出中的 pending，最终 HTML 消费也通常遍历整个结果。IceXIV 要输出全部详情，最终内容的构造/输出成本不能靠换成惰性求值凭空消失。融合或流式化可以省中间结构，但不能删除有语义身份的调用与插值边界。

但移入 call-by-need 会增加 thunk 分配、更新、blackhole、间接访问与保留环境。GHC 官方 full-laziness 文档明确警告增加共享可能增加内存驻留。对已经有约 6.54 GiB RSS 的场景，更大的长期缓存未必是好事。

### 5.4 不要照搬 GC / Arc 图

GHC 的 tracing GC 能处理有环堆图，也能通过 nursery 等机制摊薄短命对象管理。Rust 的 Arc 图如果出现“InstanceRecord → body → resolved call → 同一个 InstanceRecord”，可能形成引用环；不能直接把闭包图里的每条边都换成拥有 Arc。

实例表拥有记录、调用持有 ID/非拥有引用，会比递归持有 Arc<Record> 更稳妥。arena 也不是免费答案：其中的 String、Arc<Data> 等 Drop 必须被执行，不能直接丢弃内存区域而泄漏它们拥有的堆和引用。模块缓存、单文档临时值、最终发布对象还需要不同生命周期区域。

## 6. Typst：更接近文档系统，但语义仍不能照搬

以下均基于固定 Typst commit，不泛指其它版本。

### 6.1 Typst 的 Vm 并不是证据中的字节码 VM

`typst-eval/src/code.rs` 对 `ast::Expr` match，再递归调用 `Eval::eval`。`vm.rs` 的 Vm 保存 Engine、scope stack、flow、context 等状态；这个名字不意味着当前表达式执行使用字节码。

所以“Notist AST interpreter 慢，Typst 用 VM，因此要写字节码”不是成立的推理。树解释本身不排除良好表现，运行时表示、共享和缓存边界同样关键。

### 6.2 普通内容求值与 realization 分开

`code.rs:332–340` 的普通 ContentBlock 立即求值 body；`call.rs:72–79` 的普通函数调用先求 callee 和 args。构造一个未使用的普通内容块，也可能执行其中普通代码，与 Notist 的未挂载 opaque 内容调用不同。

Typst 另外用 `context` 等机制延迟：`code.rs:389–413` 将表达式和捕获值装成 Closure，然后包进 ContextElem。realization 才结合 styles/location、show rules 等处理内容。不能把“layout/show 还没发生”理解成“所有内容里的普通表达式都还没求值”。

对 Notist 有价值的原则是**分清内容值、延迟计算和呈现上下文**，而不是将两个系统所有延期行为一一等同。

### 6.3 Typst 的闭包把代码与捕获值分开

`Closure` 保存 SyntaxNode、defaults、captured Scope。`CapturesVisitor` 收集自由变量；调用时 `eval_closure` 以捕获 scope 为基底建立新 Vm，不把调用现场的全部 scope 泄漏进去，也不先将 captured 值递归替换进整个 body AST。

这正好对应 Notist `capture_expr` 可改进的方向。但 Typst 仍会在调用时 clone captured Scope：其 Scope 是 IndexMap，不应夸大成零成本持久化环境或 slot VM。值得学的是**共享代码 + 小环境**的结构，而不是每个细节已经最优。

### 6.4 Content 是共享句柄，而不是递归拥有的胖节点

`Content` 包装 RawContent。`raw.rs` 描述引用计数头、元素特定 data、vtable、metadata、hash；clone 共享负载，修改时 make_unique/COW。children 中是 Content 句柄，复制容器不自动深克隆每个后代负载。某些变更仍会触发 COW，整段修改也不一定免费。

内容按元素类型存储特定字段；公开字段访问既支持名称，也有直接 `u8` field ID，源码明确建议可用 ID 时优先用 ID。这与 Notist 所有参数绕过字符串 Dict 的改进方向相呼应。

RawContent 的 span 还与共享分配分开存放，但 label/location/lifecycle 等 mutation 仍可能要求唯一化。不能笼统声称 Typst 的所有元数据都外置或永远不复制。

不建议先复制它的 unsafe 自定义引用计数。Notist 可以先用安全的 Arc/COW、签名共享、源标识 intern、mount-local 元数据验证价值。

### 6.5 comemo 缓存的是明确边界，不是“每个小表达式”

- `typst-eval/src/lib.rs::eval`：模块求值带 `#[comemo::memoize]`。
- `call.rs::eval_closure`：用户闭包调用带 memoize，并接收 world、context、introspector、sink、route 等 tracked 输入。
- `Func::call_impl`：区分 native、element、closure、plugin；不是所有 native 调用都在这里自动套相同缓存。
- 结果里的 Content 可廉价共享，Closure 使用 LazyHash，缓存命中/结果复制的表示成本也受到控制。

这不是仅靠函数名和参数做“纯函数缓存”。World 读取、上下文、诊断 sink 与路径等都影响正确性。

对 Notist：当前 compile/bind 缓存只消除了模块工作，并不缓存整页求值。重复 serve 的 per-page evaluate cache 可能有价值，但首次 51,225 个不同物品输入不会因同一个函数名就命中。

若跨 batch 复用结果，至少还要处理：

1. 具体 Item/程序/实际 transform Context 的身份与版本。
2. 实际读取依赖的新版本，以及新 batch 的路径授权，而非信任旧缓存给出的能力。
3. Data view 的 backing、source binding、DataRetention 和失败/Pending 状态。
4. 每次挂载必须发生的逻辑请求、配额、依赖和诊断。

仅保存 dependency set 不够：集合丢失请求重复次数、顺序及首次配额失败位置。若这些行为需要保持，缓存必须有受限 effect/charge transcript 或限定在无这些效果的子计算；不是重用 HTML/Item 再补一个依赖集合即可。

## 7. 建议的演进路线：保语义，先减少结构成本

### 第一层：不改变求值策略的低风险改进

1. **共享不可变 FunctionDef/调用签名与 SourceId**：减少 `capture_call`、item_shell 的深层名称/签名复制。保持公共 wire/诊断投影。
2. **已验证调用的参数布局**：预先确定参数位置、optional/default 转发和实际 native/Code 路径；缺少验证证据时走现有通用检查路径。
3. **实例 ID / interned key**：保留符号身份，但在 Context 局部避免命中时仍重复格式化 String、重复查表。
4. **scope frames**：先替换整 Scope clone；后续再考虑解析到 slot。
5. **冷恢复元数据**：成功路径尽量持有廉价 checkpoint，失败时才构造完整恢复节点。不能破坏 recovery boundary。

这些是候选实验，不是已测出的收益承诺。

### 第二层：真正解决 capture-copy 的闭包式表示

概念示意，不是既定 API：

```text
checked Item / expression template     // 共享、不因每次调用改写
    + minimal captured environment    // 自由变量值，而非替换后的 AST
    = reusable content recipe

recipe + MountFrame
    = 每个挂载自己的求值、预算、attrs/origin、输出节点
```

必须注意现有约束：`evaluation_reads_the_supplied_item_tree_without_a_parallel_representation` 测试直接修改 Item 的待求值参数，要求 evaluator 读到修改，并允许实例按需检查。

因此不能静默加入另一份权威 ContentPlan/bytecode，把可见 Item 变成被忽略的镜像。可先让现有 Item/TypedExpr 的 payload 共享、挂载状态分离；若加派生执行布局，必须有严格身份/失效规则及未缓存 fallback。彻底新增独立执行 IR 是架构选择，不属于无争议微优化。

### 第三层：共享结构与较粗的增量边界

- 多次挂载共享不可变结构，但独立执行、独立 mount 身份与错误来源。
- origin expansion 考虑来源帧/持久链，避免每次对所有后代追加完整 Vec。
- per-document arena 或 compact node 存储需验证实际分配来源、Drop 和输出生命周期，先测后改。
- 跨 serve 重建的 per-page cache 最后再设计；先让结果及依赖表示适合共享，明确缓存失效和逻辑效果契约。

### 暂不优先

- 全面 Haskell 式惰性数据：会改严格 let、资源读取与错误触发时机。
- 所有内容首次展开后 memoize：会改重复挂载逻辑读取与预算。
- 所有小函数套 comemo：参数 key/验证/保留结果可能比小 native 本体更贵，且多数物品输入不同。
- 先写字节码/JIT：不先解决 owned tree、String key、签名复制和作用域，即使换 opcode 循环仍会做这些工作。
- 先换 allocator 或把 Drop 移出 Updated：前者只能作为独立诊断，后者可能只是隐藏成本，不解决结构性工作。

## 8. 不可破坏的语义与验证矩阵

| 改动 | 必须验证 |
| --- | --- |
| 捕获环境 / slots | Block、for、match 遮蔽；自由变量捕获时机；重复挂载互不污染 |
| native fast path | 按实际 FunctionId/implementation，不按 `panic`/`text` 名称；transform 保持原语义身份 |
| 内容共享 | 每次挂载的 attrs、空内容边界、instance/target、origin expansion 和错误恢复不被合并 |
| 缓存 | 请求计数、授权、依赖版本、Pending→Ready、负缓存、Data view 所有权、诊断来源 |
| 融合遍历 | content_nodes、每挂载 steps、累计 iterations/loop_work、失败时不暴露成功前缀 |
| 新执行布局 | 修改 Item 后不会跑陈旧副本；公开语义树仍是权威来源 |
| arena / 共享图 | Data/String/Arc 正确 Drop，无递归 Arc 引用环，无缓存无界驻留 |

预算还需要一个明确原则：限制约束的是语言层抽象工作，还是解释器实际步骤？如果优化改变 tick 数，不能未经决定就改变原有成功/失败边界。保守演进应先保持已有约束和测试行为。

相关既有测试：`tests/panic.rs::unmounted_content_stays_lazy_even_with_divergent_arguments`，`tests/data_bodies.rs` 的 capture/重复挂载/严格外层读取/累计预算测试，`tests/reading.rs` 的未使用内容不读资源测试，以及 evaluator 的单一权威 Item 树测试。

后续应逐项测量：无密集计时的原版 wall、低频计数、分配次数/总字节/peak live heap、origin 和签名 clone 次数、共享森林 unique/COW 分支比例。每个优化一个 A/B，完整输出一致还不够，上述错误和配额行为也要一致。上轮 Self 表不能代替这些因果实验。

## 9. 核心判断

Notist 当前最值得改的并不是“纯函数语言为什么没做惰性求值”，而是：

> **已有静态检查和共享函数体，却仍以递归代换、拥有树物化、字符串协议和多次挂载边界往返来完成大量非常小的运行时操作。**

向 GHC 学共享代码和专门化，向 Typst 学内容值的共享表示与依赖明确的缓存边界；同时保留 Notist 自己有意设计的逐挂载计算、统一语义树、结构角色、来源和预算。这样的方向比“全面惰性化”或“加更多缓存”更符合当前证据。

## 参考源码

### Notist

- `crates/notist-pipeline/src/code/eval.rs`：运行时、bind/present、调用、capture、来源、预算。
- `crates/notist-core/src/evaluation.rs`：TypedExpr / Invocation / PendingEvaluation。
- `crates/notist-core/src/item.rs`：统一 Item 及挂载边界。
- `crates/notist-core/src/definitions.rs`：FunctionId、FunctionDef、实例 key。
- `crates/notist-pipeline/src/code/mod.rs`：共享 CheckedProgram / InstanceRecord。
- `crates/notist-core/src/shared.rs`：已经存在的共享数据容器与 metrics 缓存。

### Typst，固定到 84cca406

- [表达式解释与普通/上下文内容](https://github.com/typst/typst/blob/84cca406aafe7f830ed3f37f1cc6f0c8b6b65a03/crates/typst-eval/src/code.rs#L332-L413)
- [闭包构造与 memoized eval_closure](https://github.com/typst/typst/blob/84cca406aafe7f830ed3f37f1cc6f0c8b6b65a03/crates/typst-eval/src/call.rs#L603-L748)
- [模块求值缓存](https://github.com/typst/typst/blob/84cca406aafe7f830ed3f37f1cc6f0c8b6b65a03/crates/typst-eval/src/lib.rs#L37-L97)
- [native / element / closure 调用分派](https://github.com/typst/typst/blob/84cca406aafe7f830ed3f37f1cc6f0c8b6b65a03/crates/typst-library/src/foundations/func.rs#L391-L433)
- [共享 Content 原始表示与 COW](https://github.com/typst/typst/blob/84cca406aafe7f830ed3f37f1cc6f0c8b6b65a03/crates/typst-library/src/foundations/content/raw.rs#L14-L218)
- [按字段 ID 访问内容](https://github.com/typst/typst/blob/84cca406aafe7f830ed3f37f1cc6f0c8b6b65a03/crates/typst-library/src/foundations/content/mod.rs#L169-L209)
- [realization / show rules](https://github.com/typst/typst/blob/84cca406aafe7f830ed3f37f1cc6f0c8b6b65a03/crates/typst-realize/src/lib.rs#L351-L432)

### GHC

- [Thunks, blackholes, and indirections，9.12.2 runtime 源码](https://github.com/ghc/ghc/blob/ghc-9.12.2-release/rts/Updates.h)
- [官方优化文档源码：worker/wrapper、specialisation、full laziness 等](https://github.com/ghc/ghc/blob/ghc-9.12.2-release/docs/users_guide/using-optimisation.rst)
