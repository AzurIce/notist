# 求值器重构切入点：从内容代换到共享配方与挂载

## 范围与结论

本次重新阅读当前 7ab71ca 的求值器、检查器、内容构造、扩展和相关测试。生产代码没有变化，也没有运行新的性能实验。工作区已有未跟踪的上一份 runtime 分析文档，保留不动。

结论不是“给 Item 再加一个占位变体”，而是：

1. **以 Runtime::Content 为内部表示切口，把内容值从 Arc<Vec<Item>> 改造成共享程序/模板与绑定环境的组合。**
2. **bind_content、capture_*、present_call、repeat 是同一个语义闭环，不能只重写其中一个 helper。**
3. **拆开 execute 的函数计算与挂载提交，再引入实例执行布局与 CallPlan。**
4. **第一阶段可保留完整 Evaluation.root，验证捕获和调用优化；若最终不再物化完整 Owned Item，必须进一步修改内容消费与扩展边界。**

首要目标是改变运行时组织，不是增加所有函数的结果缓存，也不是立即改写 parser、Shape、HTML 或 book/site。

## 1. 当前调用链及各节点真正职责

主要位置都在 crates/notist-pipeline/src/code/eval.rs。

```text
run:214
  ├─ bind_content(root, empty scope):240
  └─ present(bound forest):245
       └─ present_node:578
            ├─ 普通节点 → present(children)，验证结构并累计节点数
            ├─ Call → present_call:706
            │    ├─ 选择目标、检查深度、重置该挂载 steps
            │    ├─ execute(call, empty scope, children, boundary):733
            │    │    ├─ 求参数、构造 fields/Code inputs
            │    │    ├─ implementation:1166
            │    │    ├─ Code → expr(shared body, inputs):1065
            │    │    │    ├─ Content → bind_content:330
            │    │    │    ├─ For → capture_expr → Repeat:342
            │    │    │    └─ Block / If / 数据操作等
            │    │    └─ 提交边界、写 fields/target、expand、包装单节点森林
            │    └─ 再 present 返回森林:736
            └─ Repeat → repeat:635
                 ├─ expr(subject, empty scope):654
                 ├─ 每轮建立/更新局部绑定
                 ├─ expr(body, local):673
                 └─ 每轮 present，任一轮失败则不暴露前缀
```

这不是“已经检查函数还在反复重新检查”。CheckedProgram/InstanceRecord 已共享；implementation 有局部缓存，缺失目标实例时才按需检查。更大的重复组织工作是内容绑定、递归捕获、通用调用协议和结果物化。

## 2. 第一切口：Runtime::Content，以及依赖它的整条闭环

### 2.1 当前表示默认要求内容值成为拥有森林

eval.rs:110–117：

```rust
enum Runtime {
    Value(ComptimeValue),
    Content(Type, Arc<Vec<Item>>),
}
type Scope = BTreeMap<String, Runtime>;
```

expr(Content) 会在构造内容值时调用 bind_content，创建新 Item 森林；不是持有共享模板与环境。capture_expr 输入内容时又能直接 clone 森林。

建议引入 pipeline 私有的 ContentValue/BoundContent，不要一开始把解释器环境和缓存状态塞进公开 Item.pending。概念字段为：模板/程序引用、捕获环境、构造期已完成的动态片段、类别与逻辑摘要。名称与具体布局尚不固定。

第一版仍可在挂载时产生完整 Item 结果，public Analysis/Evaluation/wire 不变。这样能独立检验 capture-copy 的成本，而不同时迁移 renderer、query 和全部扩展。

### 2.2 为什么 capture_* 不能单独替换

capture_call/capture_item/capture_expr 位于 eval.rs:1208–1344。外层数据被写入 Constant/known，内容输入被写入 ExprKind::Content。于是挂载处有意执行：

```rust
self.execute(call, &Scope::new(), children, &mut boundary)
self.expr(subject, &Scope::new())
```

空 Scope 能工作的前提，就是自由输入已在前面被代换。

如果只让 capture_expr 返回共享表达式、其它不改，挂载和循环就会失去自由变量。正确的迁移单位必须同时覆盖：Runtime::Content、bind_content、For 捕获、present_call、repeat 和读取内容输入的路径。

### 2.3 bind_content 不是一个可以直接删掉的复制 pass

eval.rs:463–529 同时做了四件事：

1. 复制节点 shell，绑定 pending。
2. Splice 当场调用 expr，处理类型错误与失败，建立保留插值组。
3. 遍历 children，包括 opaque 调用的内容部分。
4. 计算绑定阶段正常节点数，并逐兄弟执行额度判断与恢复。

只把它改成 TemplateRef + Env，然后把一切推到 present，会改变构造期插值、严格数据计算和额度失败时机。

建议保留一个 construct/bind 语义阶段：已知纯静态片段引用模板；必须此时执行的 Splice 保持原时机并保存动态片段/错误；opaque Call/Repeat 保存程序引用和捕获环境。逻辑节点数、错误位置与插值边界仍需与当前一致。

第一版即使仍遍历模板，也可以不再复制 TypedExpr。进一步跳过静态片段遍历，需要静态摘要与精确边界恢复证据，不能第一步就宣称整个 bind 可以 O(1)。

### 2.4 环境必须是绑定快照，不是当前 Scope 的可变引用

Block、match、for 的词法绑定必须正确。尤其 repeat 在 eval.rs:662–672 重用 local，并不断插入下一轮循环值。如果延期内容捕获的是可变 local 地址，所有后续挂载可能读到最后一轮。

候选是不可变帧/持久环境，或按自由变量集合复制小环境。首轮可以保留名字查找，先完成闭包式表示；之后再引入 BindingId/slots。

不能为了省表达式复制而永久捕获整个 Scope；那可能保留无关 Data、内容和外层帧，恶化驻留。DataRetention 的观察和计费时点也不能因 known 变成环境读取而被悄悄跳过或增加。

## 3. 第二切口：execute 的计算与挂载提交

### 3.1 当前 execute 做了不止“执行函数”

eval.rs:918–1164 包含参数求值、optional forwarding、值验证、目标实例、native/Code 分派、内容返回校验、Item 提交、来源传播和森林封装。

内容成功分支会从 prototype 取走边界，设置 fields/target/evaluation，然后把 Item 放进 Arc<Vec<Item>>；present_call 随后解包，再 present 一遍。

这使“运行时内容值”和“已完成一个挂载的树”共用 Runtime::Content 通道，也让数据调用路径携带不适用的 prototype 参数。

### 3.2 建议的职责边界

概念上分为：

- 调用准备/执行：按原顺序求参数与验证，得到数据结果或未挂载的内容 body/recipe，以及需要提交的 fields、target 和 native/Code 信息。
- 挂载控制：管理深度、局部 steps、边界恢复、来源上下文，挂载 body，检查结构并完成一次提交。

保留每个语义调用边界，不通过删 wrapper Item 来制造节点数或来源差异。减少的是为了传递该边界而创建并立即拆开的临时 Vec/Arc。

### 3.3 必须保留的顺序

- present_call 先处理 Code 深度，再重置该内容边界 steps。
- execute 先检查 call.invalid，然后按记录顺序求参数；implementation 的按需检查发生在参数处理后。
- 构造 children 与实际挂载 children 不等价，不能为了快路径把子调用提前执行。
- 所有可能失败的操作结束前，原边界仍归恢复路径所有。
- 成功执行后的 sibling aggregate overflow 可能恢复原 fields/target；PresentCheckpoint 已优化过，不应重新引入整节点 eager clone。
- repeat 不能暴露成功迭代的前缀。

因此这次拆分不是“删掉 present 返回后的那次调用”即可；新的挂载路径必须补齐原来由那次递归完成的节点计数、结构校验和恢复行为。

## 4. 第三切口：检查产物旁的执行布局与 CallPlan

### 4.1 放置位置

crates/notist-pipeline/src/code/mod.rs 的 InstanceRecord 已持有共享 checked body。crates/notist-pipeline/src/code/check.rs:1447–1570 负责具体实例与成功检查计划复用。

建议在已检查实例旁派生不含运行时值的 InstancePlan，例如：表达式/内容模板 ID、自由变量集合、词法槽位布局、参数布局、静态字段和结构信息。不要每页每次调用重新遍历并编译完整共享函数体，也不要让计划持有另一批次的 Data 或 Env。

来源 Item/TypedExpr 仍是权威输入；计划必须来自当前值，或由明确版本身份验证。程序表拥有代码，递归调用使用局部 ID，避免 InstanceRecord → body → InstanceRecord 的拥有 Arc 环。

### 4.2 execute 的参数区是直接受益点

eval.rs:929–1018 当前每次建立目标，按名字查签名、生成 fields，Code 还生成 inputs；data.rs 的 native 再按名称取值。

CallPlan 可以保存参数位置、optional forwarding、children/comptime 的布局，以及在当前执行 Context 下可确定的实际实现。普通数据 native 可直接消费位置参数，不强制先创建公共 Dict；内容边界仍保留完整可观察 fields。

需要同时调整 code/data.rs 的参数入口，否则只是把字典构造移到另一个 helper。Data 的读取授权、parse/decode 缓存、错误来源与 Retention 不因分派优化改变。

### 4.3 不能只按 FunctionInstance 缓存 CallPlan

当前测试允许修改传入树里的参数和 captured signature。eval.rs:1999 与 2046 分别断言参数编辑、签名约束编辑对本次执行仍然生效。

因此计划身份还需要覆盖当前调用契约和执行 Context/映射等；不能仅用函数 ID/编译期实参找到旧计划后忽略本次调用。无有效证据时走通用路径，或重新派生。

PreparedCall 也不能通过提前触发 checker/错误而改变原来先求实参再出现的行为。

### 4.4 Native Construct 的消除应在这里做

eval.rs:1068–1151 的原生构造通常无需用户函数解释，却仍通过通用 Runtime::Content 和单节点森林往返。可在确认实际 target 确实是 Native Construct 后，直接使用已准备布局形成该挂载的结果。

必须保留 embed 路径规范化、原生/Code 状态、返回类别、参数验证、计费和恢复。不应按 text/strong 的名字直接绕过替换机制。synthetic_call 对手工节点和映射的支持保留作兼容通路；已有剖析不支持把它当作 icexiv 主热点。

## 5. 静态部分复用与 thunk：应在前述基础上加入，而非先加缓存

已有 known 常量在 expr:315 直接使用；check.rs:920–971 已计算简单二元表达式的 known。不要重新实现同一层常量折叠。

真正新增的能力是区分：

- 不依赖调用参数的静态程序/模板负载。
- 不依赖部分参数的纯计算，可专门化为剩余程序。
- 允许共享一次求值结果的绑定。
- 必须逐挂载执行的资源、错误与预算行为。

首轮先共享模板、签名布局和常量负载，不共享完整挂载结果。自动提升 complex_fun(8) 或建立 memo cell 需要自由变量、效果、失败、执行映射、预算和驻留边界的分析；不能只看参数依赖。

特别注意 eval.rs:289–297：comptime 模式不采用 runtime replacement。因此不能直接拿现有 constant() 执行任意运行期闭合调用，称为等价的部分求值。必须证明目标与相关语义一致。

即使纯计算结果相同，跨挂载共享也可能跳过原有 tick 或改变首次失败位置。若继续保持当前预算语义，执行计划需保留相应逻辑收费与失败点；若要改变，应先作为契约决策。

## 6. 来源、结构重写和最终输出：分开两个阶段

### 6.1 内部重构先保留外部树

保留 Pipeline.evaluate 返回 Evaluation.root: Item，使 query、HTML、book/site 与 after_evaluate 继续工作。内部采用 BoundContent/CallPlan，最终形成同样完整结果。

可以减少绑定副本、捕获代换、通用调用和临时森林；**不会消除最终 427 万个 Owned Item，也不会解决 SSG 的全站驻留。** 这阶段的性能收益必须独立验证，不宣称 300MB 已可达。

### 6.2 不再完全物化，是后续跨边界重构

eval.rs:1419 的 expand 即使链 payload 已共享，仍写每个后代。要彻底移除此类遍历，可采用模板定义来源 + mount-local 来源帧，并通过结果视图投影完整来源。

但当前 Item.origin、children: Vec<Item>、HTML 的 &Item、query/序列化和 after_evaluate(&mut Item) 都直接观察具体树。只改 expand 不可能在保留外部行为的同时让这些信息凭空存在。

后续需要 ContentView/结果句柄、按挂载的逻辑身份、按需完整 Item 投影，以及明确的编辑事务/COW。结构重写产出新结构，未改子结构可共享；不修改公共模板中的参数槽位。

after_evaluate 在完整结果后执行，当前实现为事务性 clone 候选树。第一阶段保留；后续可提供显式物化的旧 hook 适配，或迁移到内容编辑接口，不能默认绕过扩展。

## 7. 最小闭合迁移与建议次序

### 准备：固定行为 oracle，而非只保留 present_reference

当前 present_reference 只覆盖 checkpoint 版本差异，expr/bind/execute 仍共用，不能单独作为完整新执行器的正确性证明。需要可对照的旧引擎/冻结版本与独立输入观察器，比较完整结果、诊断顺序、来源、依赖和逻辑请求。

### 第一阶段：共享程序 + 捕获环境

在现有 pipeline 内引入私有 ContentValue/BoundContent。同步改造 expr(Content/For/Input)、bind_content、capture_*、present_call、repeat，必要时先拆出 execute 的挂载提交职责。保留公开 Item 输入/输出与构造/挂载分界。

验收重点：重复挂载独立、词法捕获准确、无大 Scope 长期保留、运行时不再递归复制 TypedExpr；任何代价只是从 capture 移到新 plan 编译，也必须纳入端到端。

### 第二阶段：InstancePlan + CallPlan + slots/native worker

复用已检查实例旁的计划，减少名称协议与临时字典，处理原生构造的直接提交。复杂/修改过的调用保留正确 fallback。用真实 CPU/分配栈选择优化目标。

### 第三阶段：不物化的结果视图

联合 core 的内容访问、pipeline 输出、HTML、query 与扩展边界，建立共享结构与挂载覆盖，逐步让消费者不要求完整 Owned Item。此时再改来源投影和局部编辑方式。

### 后续：有证据的计算提升或 memo

先有自由变量、效果与计费模型，再选择静态计算提升或共享结果。并不是先把所有 let 改为 thunk，也不是全局缓存 foo(args)。

## 8. 必须覆盖的验证矩阵

- tests/data_bodies.rs:170：循环遮蔽、嵌套捕获与重复挂载。
- tests/data_bodies.rs:283、322：未挂载不读、两次挂载两次读、严格外层读取。
- tests/data_bodies.rs:343、403：累计预算与失败迭代不泄露前缀。
- tests/data_bodies.rs:194、222：结构证据与未选中运行期分支的静态检查。
- tests/panic.rs:361、375：未挂载内容参数不执行，以及按实际定义分派。
- tests/reading.rs:249、273：Pending 不暴露部分结果、不运行输出扩展。
- eval.rs:1655、1817、1891：checkpoint、成功提交时机、完整摘要与临界额度。
- eval.rs:1999、2046：修改输入树与调用签名仍然有效。
- comptime/runtime target 不同、source root 与 read grant 分离、DataRetention、optional 参数转发、children 多次挂载、手工 Item/synthetic 调用、两种内容前端和全部 wire/query 投影。

每阶段再比较普通 release 的完整 wall/CPU/RSS 与分配流量。既有 execute/present/bind 调用数只指示调查区域，不提供已证明的加速倍数；最终物化节点数不下降的阶段也应如实报告。

## 最终建议

**第一刀落在内容值的运行时表示和绑定/挂载协议，而不是公开 Item 上新增一个占位节点。第二刀落在 execute 的职责与已检查调用布局。最后才推进公开内容结果的共享视图。**

这条路径可以先在 pipeline 内获得独立可验证的改进，又不把将来摆脱 Owned Item 的可能性堵死；同时承认，不物化整棵结果树是跨语言模型与消费者 API 的后续工程，不能由一个局部 helper 重构自动实现。
