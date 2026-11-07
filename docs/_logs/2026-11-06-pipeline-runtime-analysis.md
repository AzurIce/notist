# Pipeline 的程序、内容与运行时表示分析

## 结论与范围

分析以提交 7ab71ca 的实现为准。本次只阅读源码和固定版本的外部参考，不修改运行时、不启动性能实验；此前剖析数字只作既有证据，不作为本次新测量。

最重要的结论：**TypedExpr 与 Item 相互嵌套不是错误；运行时为了捕获变量而复制、代换这套嵌套结构，同时让它承担内容值、待执行配方和挂载结果，才是主要架构问题。**

不能把问题简化成“AST interpreter 慢”“需要全面惰性求值”或“多加一层 Arc”。同样也不能从源码操作次数推导某个重构必然加速几倍。

本文集中在语言核心与 pipeline，不重复 book/site 的全站驻留、模板和发布问题。后者是独立的端到端限制。

## 1. 具体是什么嵌套

当前主要关系如下，省略字段：

```rust
TypedExpr {
    ty,
    known,
    kind: ExprKind,
    source,
    span,
    shape,
}

ExprKind::Content(Vec<Item>)

Item {
    pending: Option<Box<PendingEvaluation>>,
    children: Vec<Item>,
    fields,
    attrs,
    instance,
    target,
    origin,
    evaluation,
    // ...
}

PendingEvaluation::Call(Invocation)
PendingEvaluation::Splice(TypedExpr)
PendingEvaluation::Repeat(TypedExpr)

Invocation {
    arguments: Vec<(String, TypedExpr)>,
    definition: Shared<FunctionDef>,
    // ...
}
```

从内容字面量走进去，会到 Item；从 Item 的 pending 走进去，又回到表达式和调用；调用参数也可以继续构造内容。源代码是 crates/notist-core/src/evaluation.rs:10–185 与 crates/notist-core/src/item.rs:384–407。

这是一套互递归的类型，不意味着内存里必然存在引用环，更不意味着无穷展开。有限的语法可以形成有限树，Box/Vec 也让 Rust 的类型布局成立。真正的引用环风险是在未来把闭包、实例记录和函数体的所有边都改成拥有 Arc 时才需要额外警惕。

这种设计有合理动机：内容是一等值，内容中允许调用和插值；调用边界又需要保留注解、身份、来源和结构类别。它不是无意中混入的错误数据。

## 2. 为什么嵌套本身不是问题

普通函数式语言也允许一个值持有计算：

- 闭包持有代码入口和捕获环境。
- Haskell 的构造器字段可以指向尚未求值的 thunk。
- Typst 的 ContextElem 持有用于后续上下文求值的 Func/Closure。

因此正确目标不是“结果永远不能含计算”，也不是“所有内容立即算完”。需要明确的是：其中保存的是共享程序引用、小环境与独立执行状态，还是一份会在每次绑定时被递归改写的程序副本。

当前混合了四个概念：已检查程序、绑定后的内容配方、某次挂载状态、完成的内容结果。它们经常使用同一种 Item/TypedExpr 物理结构表示，使阶段边界不够清楚。

## 3. 一次小调用的实际路径

示意程序：

```text
fn badge(name: String) -> InlineContent {
    [#strong[#text(text: name)]]
}
```

一处挂载 badge("Alice") 的主要步骤为：

1. 检查器为 badge 建立具体实例。函数体的内容字面量被转换成 Item 森林，strong/text 仍带 pending 调用；text 参数中保留 Input("name")。
2. 文档分析树中保留 badge 调用节点。它是语义边界，不是已展开的 strong/text。
3. 求值入口先 bind_content 根树，复制节点 shell 并重建相应 pending；之后 present 处理挂载。
4. present_call 建立本次内容调用的计费和恢复边界，execute 求实参并为 Code 实现建立 Scope。
5. 解释 badge 的共享 TypedExpr。碰到 ExprKind::Content，调用 bind_content，生成该次调用自己的内容树。
6. capture_call/capture_expr 将 name 对应的运行时值代入待执行表达式。相关表达式和内容结构被复制；共享 Text 等 payload 的 clone 不等于复制全部文本字节。
7. execute 在外层 badge 节点内挂载结果，保留身份、字段、来源与 Expanded 状态，并向返回后代附加展开来源。
8. present 继续进入返回内容，执行 strong/text 等构造调用，直到完成。

crates/notist-pipeline/src/code/content.rs:307–360 显示内容调用会形成带 pending 的 RExpr，materialize 再转成 Item。eval.rs:706–764、918–1151 给出挂载和执行路径。

这并不是每次重新 parse/check 函数体。CheckedProgram 和函数体已经共享；重复成本主要来自运行时内容绑定、捕获、参数准备、挂载与结果组织。

## 4. 第一类问题：捕获通过代换实现

eval.rs:1208–1344 中：

- capture_expr 遇到 Input 时，把已绑定数据放进 Constant/known。
- 遇到内容输入时，克隆它的 Vec<Item>。
- 遇到 Content、If、Block、For、MatchResult 等，递归构造表达式/内容副本。
- 通过复制 Scope、移除局部绑定，保留现有词法绑定规则。
- 空 Scope 或 known 走 expr.clone()，并不是只返回一个程序引用。

这不执行未选中的分支，但可能为了捕获而复制它。运行时捕获的成本因此与被访问的模板/表达式规模相关，不只是与自由变量数量相关。

适合的方向是 closure conversion：先确定自由变量与槽位，运行时只形成 code/recipe reference + captured values。捕获值应对应正确的绑定时机，不是在挂载时重新从调用现场按名称查找。

闭包转换不要求给用户语言添加 lambda，也不要求内容计算结果自动共享。它只改变实现捕获的方式。它还可能增加环境驻留和引用管理，具体收益需要测量。

## 5. 第二类问题：共享森林的根，不等于共享递归内容

当前 Runtime::Content(Type, Arc<Vec<Item>>) 只共享根森林；每个 Item.children 仍是 Vec<Item>。克隆 Vec<Item> 会递归复制拥有的后代；字段容器和身份中的共享 payload 则可以廉价克隆。

必须避免一个已经被旧实验反驳的推断：Arc::unwrap_or_clone 并非每次触发深复制。早期独立 A/B 中，该消费边界超过 99.98% 的观测是唯一持有；这是当时版本的限定证据，不是当前所有 clone 的覆盖计数。capture_expr 中显式的森林 clone、绑定时新建结构不在那个统计里。

可选方向有两条，可以分别验证：

- 先消除程序模板复制，最终每次挂载依然生成独立拥有的结果。
- 再使内容结果的 children 也采用共享句柄或紧凑 NodeId 存储，把 mount-local 信息独立表达。

不要一开始假定第二条一定比第一条更优。大量短命且唯一拥有的节点，盲目逐节点 Arc 化会增加分配和原子计数成本。

## 6. 第三类问题：已知小调用仍承担通用协议成本

execute 每次仍执行这些工作：

- 建立实际 target，查询实现类别。
- 遍历参数名，在签名参数列表中查找对应参数。
- 构造 fields；Code 调用另建 inputs Scope。
- 处理 optional forwarding、验证值约束。
- 查询实例实现，原生函数再按字符串从 fields 读参数。
- 内容结果经 Arc 森林和保留边界返回，再被 present 消费。

实例缓存、共享签名和小字典已存在，不应重复提议它们作为全新方案。但“缓存实例查询”不等于“编译好整个调用约定”。

refresh 既有计数仍有约 607 万次 execute。早期全量分类显示绝大部分是 native 小操作，不能把它们理解成几百万次用户函数递归。对 str、data_require_index、text 这类小操作，调用框架可能比核心运算更重；具体成本比例仍需要真实采样。

方向是已检查的 CallPlan、Context 局部 InstanceId、位置参数与直接实现分派。公开 fields 在内容语义/查询需要时保留，但不要求所有纯数据 native 都先创建临时公共字典。

实际目标可能被 transform 替换，不能按源码名字硬编码 builtin；省略动态检查也需要相应静态证据和恢复树边界。

## 7. 第四类问题：静态信息没有成为完整的执行布局

TypedExpr 已有类型、known 与 ContentShape，Context 也缓存成功的检查计划。这些都很好，但 runtime Input 仍使用 String，Scope 仍是 BTreeMap，调用仍携带名字参数。

这说明当前类型检查主要回答“合法性是什么”，还没有充分产出“以什么低成本布局执行”。可先引入 BindingId/slots、捕获布局、参数位置和原生分派信息；未必需要完整 SSA 或字节码。

implementation 在缺少实际目标特化时会重新进入 Checker，这是按需实例化/目标替换的一部分，不是每次调用都重新编译。新布局需要明确哪些调用在准备阶段可解析，哪些保留有界的按需路径。

物理调用深度还被限制为 call_depth.min(32)。换成显式执行栈可以改变物理实现，但不能未经决定就消除语言原本可观察的深度额度。

## 8. 第五类问题：来源与恢复状态绑定到逐节点修改

当前成功路径已使用 PresentCheckpoint；不能再把“每个节点都完整克隆恢复 shell”当作现状。来源文本、签名和 SourceChain 也已共享。

不过 eval.rs:1419–1445 的 expand 仍遍历后代。它通过链身份缓存共享 append 结果，减少重复分配，但没有取消对每个后代的写入。嵌套展开可能使节点被外层调用多次访问；成本可以与输出规模及展开层数共同增长。

候选方向是 definition origin + mount provenance frame，诊断/序列化时再投影完整展开链。它必须正确覆盖相对资源路径、函数内错误、外层调用栈、插值与循环，不能只是把 origin 字段删掉。

## 9. 第六类问题：状态与错误恢复混在同一种结构

Item 同时可表示普通结构、待调用、待插值、待循环、Native、Expanded、Error；通过多个 Option 和状态字段组合区分。完成的文本叶子也为其它状态承担固定布局。

TypedExpr.ty: Option<Type> 允许恢复树中保留不完整类型，这是编辑器场景需要的，不是“忘了完成类型检查”。但执行快路径如果直接消费这套恢复表示，就需要持续处理无效状态。

可以区分可恢复的语义 IR 与带验证证据的执行计划，失败部分走显式恢复路径。外部仍可获得完整语义树，不代表每条成功指令都必须携带所有恢复字段。

这种区分有维护性与布局价值，但引入派生计划必须处理输入/Context/扩展变化后的失效；不能出现另一份陈旧权威程序。

## 10. 第七类问题：预算与执行路径耦合

expr 和 execute 调用 tick；每个内容挂载重置局部 steps，循环 iterations/loop_work 又累计；内容节点额度和失败恢复也有独立规则。

因此，内联、常量传播、删临时节点、融合遍历，不一定是当前契约下自动合法的优化。输出文本相同，逻辑收费、错误顺序或恢复边界仍可能不同。

需要明确一份操作语义/逻辑计费规范，说明：什么必须发生、顺序和次数是什么，优化计划怎样保持它。可以在某些优化块携带 charge 证据，但超额时仍须定位正确的失败点。精确计费不是免费的；若选择改变额度语义，应作为显式版本决策。

这解释了为什么借用 Haskell/GHC 的内联规则时，不能只依据“表达式是纯函数”就直接删减工作。

## 11. 第八类问题：静态管线仍有可改进处，但不是首要热点

Parse/Lower/Resolve/Shape/Materialize 分阶段本身合理，不应为了减少遍历就混成一个大函数。各阶段保留不同领域信息，有助于类型、结构与注解的正确性。

可以调查的实现问题包括：

- materialize 从借用的 RExpr 构造 Item，克隆 pending 等负载；无 inspect 的消费式路径可能避免部分副本。
- materialize 的注解分组虽排序，但每组重新扫描内容序列；在大量兄弟节点和注解的构造负载上存在 O(A×N) 最坏路径。尚无证据说明它是 icexiv 主热点。
- TypedExpr 保留 known 与原表达式，为编译期信息和调试服务；派生执行表示可避免运行时持续承担完整检查产物，但不能声称每份 known 都复制一套大 payload。

既有 refresh 中分析约占 coarse 组装 9.3%，Data parse 缓存也有效。不能忽略它们，但不应将主要优化投资转移到 parser、JSON 或模块缓存上。

## 12. 内容的执行时机不能概括为“全部惰性”

当前语义至少有三个维度：编译期计算、严格运行期数据计算、挂载时内容计算。

- Block 顺序求值 let 初始化式。
- if 运行时只执行选中分支；已知编译期条件也可能在 checker 中折枝。
- 普通内容构造会绑定内容，opaque 内容调用不立即执行。
- for 被捕获成延迟 Repeat，挂载时执行 subject/body。
- bind_content 遇 Splice 会立即调用 expr，可能触发严格数据计算。

因此，“把 bind 全部删除，把所有计算放到 mount”不自动等价。新的 Recipe 表示需要表达正确的构造期/挂载期分界；必要时仍保留两个阶段，只是不再复制程序来实现它们。

tests/data_bodies.rs:283–339 给出关键契约：未挂载循环不读；同一循环两次挂载读两次；外层严格读取可以只发生一次。物理字节/解析可以缓存，但逻辑请求仍分别发生。

## 13. Typst 对相同问题的做法

参考固定 commit 84cca406aafe7f830ed3f37f1cc6f0c8b6b65a03。

### 捕获

Closure 保存 ClosureNode、defaults、captured Scope 等；CapturesVisitor 遍历语法确定捕获，调用时从 captured scope 建立 Vm，再绑定参数、解释原 body。它不是每次把参数值代进整个 AST。

也不要夸大：Typst 在创建闭包时仍会做捕获遍历，调用时也 clone captured Scope，并有名字参数处理；不是无分配 slot VM。对 Notist 有价值的是“程序和环境分开”，不要求逐行模仿。

### 内容与延迟计算

普通 ContentBlock 会求值 body；普通函数调用先求 callee/args。其执行时机与 Notist 未挂载 opaque 内容不同。

需要上下文延迟时，Contextual 表达式显式构造带 captured Scope 的 Closure，再装进 ContextElem。它同样让内容携带计算，但携带的是闭包，不是为本次绑定递归替换过的整个内容程序。

### 结果共享与缓存

Content/RawContent 是共享、按元素种类存储的句柄，修改走 COW；节点不是全都内联一套 pending、解释器状态与最大 payload。

eval_closure 的 comemo 边界接收 tracked world/context/introspector/sink/route；native/element 路径并非自动全部采用相同缓存。不能把它描述为“按函数名和参数全局 memoize”。

Typst 说明 AST 解释器也可以有良好结构；它没有证明我们必须改字节码。更不能直接继承其求值结果共享规则，因为逻辑读取与预算契约不同。

## 14. Haskell/GHC 对相同问题的做法

参考 GHC ghc-9.12.2-release。

### 程序和运行时堆分开

Core 表达程序，包含变量、应用、lambda、let、case 等。后续 STG 区分函数/延迟闭包与数据构造；常规编译路径再生成更低层代码。运行时不会对每次绑定继续保存并改写整棵 Core AST。

STG 源码明确以 let-closure x = [free-vars] [args] expr 描述闭包；自由变量在闭包负载的固定位置，参数由调用布局提供。运行时 closure 的 header/info table 定位代码及布局，payload 存放值引用。

### 数据包含计算完全合法

Haskell 的树/列表字段可以指向 thunk。thunk 也是 code + free variables；被需要时进入代码，常见的 updatable thunk 在成功后指向结果，使后续需求共享计算结果。不是所有函数闭包都 updatable：STG 有 ReEntrant、Updatable、SingleEntry 等类别。

这直接否定“内容中嵌计算本身就是错误”的判断。关键是运行时表示与共享/进入协议。

### 对 Notist 不能照搬的部分

Haskell 的纯值共享与 Notist 每次挂载重新执行，不是一种契约。一个被共享的动作描述不意味着动作执行结果共享；可以用这一点理解 Recipe，但不要求给 Notist 添加公开 IO monad。

GHC 的快还来自已知调用、严格性/需求分析、worker-wrapper、专门化和中间分配消除；不是仅仅因为惰性。反过来，thunk、更新、间接引用、GC 与环境保留都有成本，增加共享也可能增加驻留。

GHC 的 tracing GC 能管理一般堆图；Rust Arc 需要明确所有权和避免环，不能照抄图结构就默认正确。真正需要共享的是程序和必要的值，不是无限增长的结果缓存。

## 15. OCaml 提供的补充对照

OCaml 通常是严格求值，却同样采用闭包环境与已知调用优化。5.3 的经典 closure 转换路径计算自由变量，分配环境偏移，并把自由变量访问改写成环境字段访问。

因此，“共享代码 + 捕获环境”不是 Haskell 惰性专有技术。Notist 完全可以保留当前严格数据/延迟内容组合，同时借鉴这套实现组织。

编译器做闭包转换时也会遍历、代换 IR；差别在于这是编译阶段为代码布局做的工作，不是每次运行时绑定都重新复制同一份程序。不能把所有形式的 substitution 都一概判为低效。

## 16. 建议的目标模型与验证顺序

概念模型，不是最终 API：

```text
CheckedProgram / ContentTemplate    共享代码与静态结构
CapturedEnv                        当前绑定的自由变量值
Recipe                             模板引用 + 捕获环境
MountFrame                         本次身份 / 来源 / 预算 / 执行上下文
ContentResult                      最终内容、诊断与依赖
```

同一 Recipe 可以重复挂载，MountFrame 与逻辑执行独立；静态子结构和程序可共享，不默认共享结果。构造期必须发生的 Splice/严格 let 仍发生在原来的时机。

不建议用 Arc 连接每个实例记录和包含自身调用的函数体；程序表拥有代码，运行时持有局部 ID/适当句柄，并且 ID 明确归属 Context/revision。公开 Item 可以继续是完整投影，派生布局必须严格失效。

验证顺序：

1. 明确构造/挂载/读取/失败/计费语义，以现有 oracle 和测试覆盖为基础。
2. 原型化共享模板 + 显式捕获环境，先保留拥有的结果树，验证消除 capture-copy 的真实收益。
3. 原型化 slots + CallPlan，验证小 native 和构造调用的通用协议成本是否实降。
4. 再根据 live heap/CPU 栈选择紧凑内容、来源帧、遍历融合与更低层执行器。

每项记录完整语义结果、请求/诊断/预算边界，以及无插桩端到端 wall、CPU、分配与 peak live heap。已有数百万调用与约 1.98 亿次整进程分配支持调查方向，但不能替代这些因果实验。

## 外部参考定位

以下是源码定位，不是跨系统吞吐比较。链接以纯文字记录。

- Typst 普通内容块与 Contextual：crates/typst-eval/src/code.rs:332–340、389–413。
  https://github.com/typst/typst/blob/84cca406aafe7f830ed3f37f1cc6f0c8b6b65a03/crates/typst-eval/src/code.rs
- Typst 捕获与闭包执行：crates/typst-eval/src/call.rs:603–748。
  https://github.com/typst/typst/blob/84cca406aafe7f830ed3f37f1cc6f0c8b6b65a03/crates/typst-eval/src/call.rs
- Typst 函数分派与 Closure 结构：crates/typst-library/src/foundations/func.rs:391–433、793–805。
  https://github.com/typst/typst/blob/84cca406aafe7f830ed3f37f1cc6f0c8b6b65a03/crates/typst-library/src/foundations/func.rs
- Typst 共享内容与 COW：crates/typst-library/src/foundations/content/raw.rs。
  https://github.com/typst/typst/blob/84cca406aafe7f830ed3f37f1cc6f0c8b6b65a03/crates/typst-library/src/foundations/content/raw.rs
- GHC STG 闭包布局：compiler/GHC/Stg/Syntax.hs:194–197、304–330、409–448。
  https://github.com/ghc/ghc/blob/ghc-9.12.2-release/compiler/GHC/Stg/Syntax.hs
- GHC runtime thunk/update：rts/Updates.h:17–59。
  https://github.com/ghc/ghc/blob/ghc-9.12.2-release/rts/Updates.h
- GHC runtime closure：rts/include/rts/storage/Closures.h:95–108。
  https://github.com/ghc/ghc/blob/ghc-9.12.2-release/rts/include/rts/storage/Closures.h
- GHC 优化机制：docs/users_guide/using-optimisation.rst。
  https://github.com/ghc/ghc/blob/ghc-9.12.2-release/docs/users_guide/using-optimisation.rst
- OCaml 经典 closure 转换：middle_end/closure/closure.ml:874–884、1243–1292。
  https://github.com/ocaml/ocaml/blob/5.3/middle_end/closure/closure.ml
