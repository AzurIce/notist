# 从阶段耗时到架构与实现：性能根因分析

## 范围与证据

分析对象为 `214de5fec5ea796a047024510566168e1fa81d08`，与构建耗时流程图相同。本轮不修改生产源码，不实现优化，不新增性能计时。下面区分三类结论：

- **实测**：已有完整负载 profile 的阶段、Self、调用次数、系统调用与完整输出清单。
- **实现事实**：当前源码确实执行的复制、查找、验证与发布操作。
- **优化候选**：基于前两者提出的改法；尚未获得优化后收益，不能将被测范围当成可全额消除的时间。

主阶段统一使用流程图的 detail 探针三轮均值；求值 clone 使用独立 evaluator 补充探针，明确另列。两批不能直接相减。原有 108 次完整构建的字节校验结果不变，本轮只派生结构统计。

### 当前成本分布

| 完整负载 | 图中分析 ms | 图中求值 ms | 求值占 detail 总计 | detail 内总计 ms |
| --- | ---: | ---: | ---: | ---: |
| XIV，51,739 页 | 716.309 | 116.489 | 3.31% | 3,519.265 |
| 无模块正文，521 页 | 132.515 | 287.533 | 61.78% | 465.420 |
| 共享图，521 页 | 213.335 | 531.605 | 65.51% | 811.520 |
| 私有图，521 页 | 429.115 | 541.515 | 51.73% | 1,046.821 |
| 宽导出，521 页 | 330.180 | 1,061.749 | 71.00% | 1,495.322 |

这是两类不同问题：XIV 是大量小文档、外部数据和文件发布的吞吐；正文／模块站是内容树解释和装配。不能指望同一个模块缓存优化同时解决两类大头。

还应纠正负载名称造成的直觉：宽导出每个详情有 32 个表格行，共享／私有／无模块详情为 13 行；补充探针 execute 次数分别为 105,031、62,119、62,023、40,519。它不是与共享图“输出工作完全相同，只增加了通配符”的实验，不能用两者秒数差声称通配符绑定成本。

## 一、架构总判断

现有职责划分本身是合理的：

1. core 保存语言值、内容 IR、来源和能力接口；pipeline 做纯前端、检查和求值。
2. Vault／Environment 处理宿主资源和词法环境；SSG 负责跨页面路由、链接、模板与产物。
3. 发布模块只接收完整产物，文件系统替换属于 CLI 宿主。

问题主要不是 crate 划分过多，而是**边界之间携带的所有权过重，缓存复用的层次偏低**：

- 模块源码编译可以复用，但文档仍重建宿主图、注册环境和 checked 实例。
- Analysis 必须不可变，但每一轮递归都重复制整个子树并非这个契约所要求。
- Runtime 内容使用 Arc，却在进入可变呈现边界前多次重新深拷贝，抵消了一部分共享收益。
- 完整产物后发布便于失败隔离，但也要求每次重建写出和删除几万文件。

应先改变“工作如何被拥有和重复”，而不是先换模板库、放宽额度或给整个 Vault 加全局锁并行。

## 二、求值：深拷贝被递归和两遍处理放大

### 2.1 实现中的实际数据流

关键位置：

- `crates/notist-pipeline/src/code/eval.rs:147`：run 借用 Analysis 的 root／program，但先执行 `program.clone()`。
- `:172`–`:177`：bind_content 生成新的拥有森林，再将它借给 present。
- `:420`–`:431`：先 `node.clone()`，随后重新 capture pending，再递归生成 children，覆盖刚复制的 children。
- `:496`–`:498`：present_node 再复制整个 Item，然后递归呈现复制出来的 children。
- `:598`–`:602`：一次内容调用分别复制 children 与整个 prototype Item。
- `:828`–`:832`：execute 又将 children.clone 放进局部 Scope；即使实际目标是原生 Construct，也先付出这个成本。
- `:948`–`:974`：prototype 的 fields、pending、target、evaluation、children 随后被重设。
- `:1028`–`:1144`：capture_call、capture_item、capture_expr 同样存在先整体 clone、再递归替换内部字段的写法。

也就是说，一份语义内容在到达最终输出前，可能依次成为绑定树、呈现输入树、调用 children、调用 prototype、作用域内 children，再进入最终输出树。它们不是六棵同时总是存在的固定副本，但代码允许同一子树被反复复制和回收。

### 2.2 不是所有 clone 都一样贵

`crates/notist-core/src/item.rs:382` 的 Item 派生 Clone 会递归复制 `Vec<Item>`，并复制拥有的 pending、instance、origin 等。

但 fields／attrs 的 Dict、语言 Array、字符串 Text 已经分别共享 Arc 存储，见 `crates/notist-core/src/shared.rs:12`、`:110`、`:261`。不能把它描述为“每个字段的全部数据都深拷贝”，也不能根据搜索到的 clone 行数排优先级。

当前昂贵组合是：**拥有的 children／表达式树，加上每个节点拥有的来源路径、函数身份与待求值记录**。即使 Dict 很便宜，树结构与元信息的重复分配仍然存在。

### 2.3 为什么深度会放大成本

对于每个节点都执行一次完整子树 clone、然后递归处理 children 的普通分支，结构复制量近似：

`∑v |subtree(v)| = ∑u (depth(u) + 1)`。

因此不是天然只做 O(N) 节点工作：深度为 D 时可到 O(ND)，链状树最坏可到 O(N²)，平衡树约 O(N log N)。pending 内的表达式复制还会增加工作。当前完整站点没有用深度扫描实验测出一条二次增长曲线；这里是代码结构的复杂度风险，不是新的基准结论。

内容计数也可能重复遍历子树，但独立补充探针中宽导出的 content_count 约 3.934 ms、valid_structure 约 4.458 ms，远小于已测 clone。不能因为它也具有重复遍历就先删除安全检查。

### 2.4 实测支撑与边界

独立 evaluator 补充探针中，所选七类 clone 的 Self 合计：无模块 142.811 ms、共享 240.619 ms、私有 243.889 ms、宽导出 487.923 ms。宽导出其中 Item clone 249.654 ms、prototype clone 92.688 ms、children clone 64.970 ms。

补充探针相对同批 detail 本身增加约 7.5%–10.6% wall；这些时间还包含必要复制，**不能声称优化后必省 488 ms**。但它足以证明：拷贝是值得优先处理的真实热点，而不是只凭代码风格推断。

### 2.5 优化顺序

**优先：元信息复制与树结构变换分开。** 对需要重建 children 的节点，直接构造只包含所需元信息的节点壳，不先复制整棵 children；对将重建的 pending，不先复制旧 pending。保留显式字段构造，避免以后新增 Item 字段时被遗漏。

**随后：让 present 消费新构造的绑定森林。** run 内部的 forest 已经是新产物且之后不再使用，没有必要只提供 `&[Item]` 再复制一遍。可以消费拥有 Vec 并移动节点／children。借用 Analysis 的公开语义仍保持；改变的是内部中间结果的流动方式。

**再处理 capture。** 根据现有实现，空 Scope 没有可替换的自由输入；可以验证空 Scope 的快捷路径，或只重建确实被替换的分支。不能在优化 capture 时提前解释未选分支或未挂载内容。

**架构方向：共享不可变模板，挂载时才创建独立可变实例。** 单次拥有的 Arc 森林优先 unwrap；多次挂载的内容仍需隔离 origin、target、evaluation 和错误节点。不能把 Analysis 原树直接传给可变输出 hook，也不能对共享节点直接追加来源链。

必要回归：Analysis 不被回写、一个内容值重复挂载相互独立、不挂载的内容不执行、未选分支不读资源、每个挂载的步骤预算与累计循环预算不变、错误恢复仍按原边界、来源展开链与 `NeedResources` 保持。

## 三、求值：通用调用路径在每个节点重做装配

### 3.1 无模块不等于没有函数调用

`crates/notist-pipeline/src/code/content.rs:359` 将检查后的普通内容调用放入 PendingEvaluation::Call。`#table`、`#row`、`#cell`、`#text` 等本身就是构造调用，所以无 notc 模块的负载仍有 40,519 次 execute。

`eval.rs:759` 的通用 execute 每次都会：

1. 求值和检查参数；线性寻找签名中的对应参数。
2. 构造 fields Dict 与 inputs BTreeMap；处理 Optional 的包装与缺省值。
3. 分配／复制目标 FunctionInstance，查找或安装已检查实现。
4. 对原生构造或 Code 体分派，再装配统一内容边界。

统一的语义有价值，但原生 Construct 不使用 Code 体的局部 inputs 表，当前仍构造这张表，并克隆 children 给它。宽导出 execute 剩余 Self 约 250.421 ms，表明 clone 之外仍有装配成本；这个 Self 还含其他未细分操作，不能全归给 BTreeMap。

### 3.2 可以具体改什么

- 将“必须执行的参数求值／类型与可选参数语义”和“仅 Code 目标需要的局部 Scope 构造”拆开。
- 原生快速路径须在 extension mapping 选出**实际目标**后决定，而不是看到源 ctor 属于 builtin 就跳过替换。embed 正规化、类型检查、来源、预算、target 身份都保留。
- 检查后的参数可以引用稳定的参数槽位，避免每次用字符串在签名中线性查找；内部 Scope 可再考虑槽位数组。这比把所有 Dict 换成 HashMap 更有针对性。
- `FunctionInstance::key()` 在 `crates/notist-core/src/definitions.rs:279` 每次格式化身份与 comptime 实参，再对 String 做 map 查找。可考虑内部稳定 InstanceId／已规范化的键；必须保留 NaN、负零等浮点位级 key 语义，不直接改用不正确的 f64 相等性。

槽位、key 格式化和局部表分配没有独立计时，排在已确认的重复树拷贝后；不能为每项预填节省秒数。

### 3.3 已检查代码不应在每次调用时重新拥有一份

`crates/notist-pipeline/src/code/mod.rs:18`–`:28` 的 CheckedProgram 使用拥有的 InstanceRecord，其中 definition 和 body 也是拥有数据。

`eval.rs:156` 克隆整份 CheckedProgram；`:1023` 又在每次 implementation 命中后返回 definition.clone／body.clone。模块源码已经编译命中，并不会消除这两层复制。宽导出补充探针中的 body clone 约 40.379 ms、definition clone 约 9.902 ms。

适合的表示是不可变的共享实例记录／checked body，加上每次求值独立的运行状态和新增特化 overlay。只改成返回 `&TypedExpr` 未必可行：执行 body 会递归可变借用同一个 Evaluator，并可能安装新特化；共享拥有句柄可以解除这种借用冲突，而不用复制整个 body。

初始 program.clone 和最终 program 回收尚未独立计时。eval.run 的 Self 不能全算作 program.clone。共享记录也不能偷偷将 Data retention、诊断、资源依赖、预算或运行结果变成进程全局状态。

## 四、模块复用：编译缓存有效，但宿主图和检查层没一起复用

### 4.1 命中之后仍有哪些工作

`src/environment/loader.rs:239` 每份文档建立新 Graph；visit 的去重表是该 Graph 的局部表，不是整个站点共享表。

源码字节在 ResourceSession 内已固定，CodeModule 又通过 comemo 共享；但 Graph 仍遍历依赖、取得拥有的 source String、构造 BindingSpec、调用 link、向新的 Context 注册模块。

| 负载 | compile 调用 | 实际 compile miss | Context install 调用 |
| --- | ---: | ---: | ---: |
| 共享图 | 5,192 | 18 | 5,192 |
| 私有图 | 5,120 | 5,120 | 5,120 |
| 宽导出 | 3,072 | 6 | 3,072 |

次数来自同批 detail 的模块计数器，不是把递归 Graph.visit 的 Inclusive 相加。共享图 environment.document 累计约 25.242 ms，宽导出约 49.762 ms；共享图不是又编译了 5,192 次源码。

此外，`crates/notist-core/src/registry.rs:41` 的注册会再次 validate_module 并建立函数名索引。即使 DefinitionModule 共享，注册索引和环境内校验也不一定共享。

`Checker::new` 每次文档创建空 CheckedProgram；`check.rs:1447` 的 ensure_instance 只在这一份程序中去重，`:1474` 还先克隆 CodeBody，再检查成 TypedExpr。因此“源码编译缓存”“导入名字绑定缓存”“checked 特化复用”是三件事。

### 4.2 架构优化候选

- 批次共享加载过的不可变模块／注册索引；文档仍拥有各自的词法 import scope、root、配置和扩展选择，不能把全部模块合并成环境全局名字。
- 对安全可复用的 checked 实例再研究 Context 依赖键：不能只用 FunctionId 或源码文字当键。依赖实现、词法 scope、comptime 实参、资源字面量根、前端身份和诊断来源都可能影响结果。
- 不带可缓存契约的自定义前端／扩展，不能默认当作纯函数跨文档或跨轮 memoize。
- 缓存限定在固定输入批次／明确计算依赖，不恢复已经移除的全量版本扫描机制。

这部分收益的直接目标是环境和检查成本；不能把当前一秒多求值都算成这个缓存的收益。私有图 5,120 个模块有不同规范身份／来源，本身确实需要真实工作；不能仅凭文本相同合并，破坏相对路径与错误来源。

## 五、XIV 的分析和 HTML 阶段：重复查找同一个文档身份

### 5.1 实测是宿主成本，不是五万次磁盘读

`src/vault.rs:171` 的 ensure_environment 以路径和源码文字查表；首次建立 Environment、Arc 文本、登记输入并写入嵌套 BTreeMap。

XIV 对应的 detail 时间：

- ensure_environment Inclusive：295.071 ms，其中 Self 144.814 ms、Environment.document 81.790 ms、输入文本登记 68.466 ms。
- environment_snapshot：103,478 次，共 94.171 ms。
- 后者一半用于求值入口（46.580 ms），另一半用于 HTML 绑定入口（47.591 ms）。

`Vault::evaluate` 和 `html_registry_and_environment` 都只有 path／source，因此必须重新找到文档先前使用的 Environment，见 `vault.rs:348`、`:316`。SSG 虽然始终处理刚分析过的同一份文档，仍走通用反查接口。

这也说明阶段名不能代替原因：图上 XIV 求值 116.489 ms，单独解释器 eval.run 只有 35.517 ms；其中 46.580 ms 花在查环境。环境句柄优化的收益会同时体现在“分析、求值、HTML”三个节点，而不仅一个函数。

配置文本只实际有界读取一次；parse_config 仍执行 51,739 次，累计约 23.992 ms。只缓存 TOML 解析，即使完全消除该范围，也不到 detail 全程的 0.7%，不能把 295 ms 全归给配置解析。

### 5.2 该优化的接口形状

宿主分析时保留一个受该 Vault／批次约束的文档句柄：规范路径、共享源码和确切 Arc<Environment>；后续求值与 HTML 绑定消费这个句柄，不再从字符串反查身份。

SSG 目前在 `build.rs:484` 保存 `(route, text, analysis)`，可以在宿主层携带该关联，而不是将 IO Environment 塞进纯 pipeline::Analysis。句柄必须验证 belongs-to-batch／pipeline 配置等关联，防止把别的 Vault 的环境配给该 Analysis。

配置解析结果可以按本轮固定文本共享；共享模块图可以与每文档词法 root 分离。不能简单把环境 map 改成只按路径取当前版本，或把所有无 import 文档的整个 Context 指向同一 root。

`tests/vault.rs` 中 source snapshot 与 HTML 绑定环境的回归说明，这个关联不仅是性能缓存，还是语义正确性要求。

## 六、源码缓存：已不读盘，仍可重复分配字符串

`src/resource_session.rs:283` 的 Resources::read 将缓存 Arc<[u8]> 变回 Vec；默认 `Resources::source` 在 `src/resources.rs:62` 将 Vec 作为 UTF-8 String 使用。

所以“命中输入缓存”消除了后端 IO，却没有自动消除 `bytes.to_vec()`。Graph::visit 每文档查询同一个共享模块时，仍可能重新构造 source String，然后调用编译缓存。

可考虑在 session 内缓存经过 UTF-8 验证的 Arc<str>，给内部加载器使用共享文本入口，保持外部需要拥有 String 的调用语义。文档来源与模块来源都已使用 Arc，数据通路不必在中途降回拥有 Vec／String。

但不能为省这个复制而绕过请求计费、依赖观察、根能力隔离、有界读取和首次错误缓存。源码共用不等于授权数据读取。共享／宽导出有界后端读取本身各约 1.13 ms，继续删除首读安全检查不是当前重点。

## 七、准备扩展：全量传输和重新解析是当前协议的成本

### 7.1 XIV 约 770 ms 准备时间是什么

detail 中 stdio.process 约 511.268 ms，Rust JSON decode 约 120.257 ms；process 已包含子进程、管道传输和等待，不能再叠加 Python cProfile 的总耗时。

当前协议 `SiteInput` 为文档源码字符串和 JSON data：

`CSV → Python 对象 → Notist 属性源码＋JSON 响应 → Rust SiteInput → Notist 前端 → Page 属性 → Tera 值`。

XIV 的 scripts/items.py 在 add_document 中把每项属性单独 json.dumps，再生成 `@!(...)`。宿主仍必须真正解析每个生成文档。最终 item 模板使用 `site.data.items[page.attrs.item_id]` 生成正文，所以普通 Item 树较小，但完整输出正文并未省略。

这说明该架构对“海量仅有元信息的文档”有固定解释成本；不过直接提供结构化页面、跳过文档前端，是改变输入／消费者模型，不是本轮推荐默认采用的等价微优化。

### 7.2 可以保持模型的改进

- `crates/notist-ssg/src/extensions.rs:124` 对候选 input.clone，`command.rs:53` 为 Request 再 clone；后者可使用只借用的序列化请求，保持 JSON 协议不变。
- 但本 XIV 只有一次生成扩展，这两次 clone 发生在大批页面生成前，输入很小。不能看到这两行就把整个 770 ms 当作重复 input 深拷贝。
- 最后一个扩展执行后的完整 validate 和 prepare 末尾 validate 检查的是未再变化的同一 input；随后 build 又为每页构建 Route。成功的一扩展 XIV 路径因此构造约 3 × 51,739 = 155,217 份 Route。
- 可让验证产物携带已经构造的路由／输出集合，由 build 消费，省去构造后丢弃再重建。每个扩展之后的校验仍要保留，零扩展时也必须校验，不能只保留最后校验而放行非法中间状态。

路径验证与重复 Route 的成本没有独立计时，只能列为明确存在的重复工作，不能把准备阶段扣除 stdio 的全部余量都归给它。

更换 Python、原生化 CSV 扩展、生成页面新协议、改变 site.data 组织都属于另一类架构／消费者实验，需要另行授权、单独归因；当前生产内核不应为某一个消费者走旁路。

## 八、共享上下文和逐页模板：已有共享，热点分布也不相同

### 8.1 不能重复做已经做过的优化

`crates/notist-ssg/src/build.rs:44` 的 TemplateContext 一次构造全站页面值、目录值与 site.data。逐页仅替换 page／assets，在目录变化时替换 directory。

锁定的 Tera 2.4.0 在 value/mod.rs 使用 `Map(Arc<Map>)`、`Array(Arc<Vec<Value>>)`，因此 `values.pages[i].clone()` 分享 map；build.rs 的指针相等测试也直接验证这一点。Context 并没有为每页重新复制整站 data，也没有每页重新加载模板。

全站 pages.clone 仍会复制一次 Value 句柄向量，但不递归复制每个页面 map；不能将它与 Item 的派生 Clone 混为一谈。

### 8.2 当前真正付费的部分

XIV detail：共享上下文 178.122 ms；其中 page 值构造约 51.421 ms、site.data 转换约 94.773 ms。逐页阶段 450.606 ms，包含 render 274.398 ms、输出登记 127.634 ms、上下文约 36.361 ms，其余是循环与未细分工作。

site.data 的宿主 JSON → Tera 转换只执行一次。保留原数据模型时，优先研究转换／持有方式，不应说“把 site.data 移出循环即可省掉它”。原始 JSON 回收与 Tera 值回收也付出真实成本，但前者不是语言 Data API。

`src/publication/mod.rs:55` 的产物冲突检测已经通过祖先查询和 ordered range 避免了全 map 扫描，是 O(depth × log N) 级别。当前输出登记约 128 ms 说明路径分配与多次树查询的常数仍值得研究；不是重新发现了 O(N²) 扫描。

可选的等价改法是先规划输出路径，建立统一冲突索引，再把字节登记到已经验证的槽位；还需覆盖静态资源、组件、保留路径、文件／目录祖先冲突，不能删掉冲突检测。

Tera render 内部生成 Vec，再用 `String::from_utf8` 接管它；外部 `html.into_bytes()` 再转回 Vec 也接管同一分配。改 `render_to(Vec)` 主要可能省 UTF-8 检查或改变 buffer 策略，**不是省掉一次全 HTML memcpy**，也没有依据认为能消除整段 274 ms。

## 九、发布：主要是文件数量，不是两次 rename

### 9.1 当前架构成本

`crates/notist-cli/src/publication.rs:177` 对每个产物执行路径检查、create_dir_all、std::fs::write；成功替换后 backup 的 TempDir Drop 删除旧整站。

XIV 清单有 51,744 个文件，共 114,923,532 字节。这些是产物逻辑字节，不是进程 RSS。detail 测得写暂存 424.766 ms、清旧目录 356.911 ms；其余检查、目录准备、两次 rename、临时句柄清理合计约 0.151 ms。没有单独量出 rename 时间。

strace 约五万次 write、unlinkat、mkdir 为独立旁证。std::fs::write 不包含显式 fsync，因此也不能把这些数解释成介质持久化吞吐。

### 9.2 最直接、风险较小的改法

清单中只有 **5 个不同的直接父目录**，包含祖先和根也只有 **6 个目录**，却按 51,744 个文件调用父目录 create_dir_all。

先规划所有目录并创建一次，或在暂存目录生命周期内缓存已创建父目录，可删除大量无效 mkdir／metadata 探测。该数是目录结构统计，不是声称 syscall 总数会精确降成 5，也没有单独测出这些无效探测占 424.766 ms 的多少。

所有相对路径仍必须拒绝空路径、`..`、根、前缀及非正常 component；验证失败不安装新目录。可以将路径与祖先冲突预检提前，但不能因为路径来自内部 BTreeMap 就盲信它。

### 9.3 架构级改法与明确代价

- 内容哈希／不可变旧文件复用，可研究减少无变化文件的字节写出；需要额外的可信内容清单、旧输出变化处理与完整失败回退。
- 使用 hardlink 时不能继续无条件写入已链接 inode，否则会修改旧站；reflink 也有文件系统和平台条件。完整新目录即使复用了文件，仍有每文件创建链接及旧链接删除成本，不会自动消除 357 ms 清理。
- 异步删除旧目录可以缩短“新站可见”的等待，但完整构建／清理生命周期未必变快，磁盘和后续重建也会承受成本。必须分别报告 time-to-visible 和 total cleanup，不应只从计时里移走删除。
- Linux 原子目录交换主要改善可用性／失败语义，不是当前主要提速点；两次 rename 所在余量本来就很小。

本次数据全部是已有旧输出的成功重建。首次构建没有这份旧目录删除成本，不能直接套用 356.911 ms。

## 十、全站持有、潜在复杂度和并行化

### 10.1 全站阶段屏障既有必要，也有可改的表示

`build.rs:467`–`:488` 先分析全部文档，再逐个求值，保证生成／编辑后的所有文档文本先固定，后续 read_text 不依赖页面顺序。这条输入语义必须保持。

跨页面链接、锚点和模板 site.pages 也确实需要全站信息，但不要求一直保留所有中间树与宿主索引。HTML 阶段当前用 `by_source: Path → &Document`，使元信息查找与整个 Document 生命周期耦合。

可拆成轻量路由／ID 元信息索引与待渲染树，树在其最后一次使用后释放。页面 HTML 若是 site.pages 的公开值仍需保留，不能因 streaming 删除它；也不能擅自把所有 before_check／after_evaluate 的跨页面调用次序改为另一套。

更大的变体是可失败回收的 staging sink／spool，逐步接收最终字节、全部验证成功后才安装目录。这会改变当前 BuildOutput 拥有整站 bytes 的输出接口和 IO 职责，适合另立设计，不是一次局部 clone 修复。仅把 Drop 提前或移到后台不等于减少总工作。

### 10.2 现在不大、未来可能变大的链接路径

`build.rs:248` 与 `:312` 优先按源路径查 map；如果引用 HTML 输出路径或目录别名，会退化为 `documents.values().find(...)`。大量此类引用时可出现 O(links × pages)，缺失引用也可能触发扫描。

一次建立 `by_output`／目录别名索引可以改善复杂度，仍保留源路径优先、origin 定义位置、query／fragment、锚点验证与警告／错误级别。XIV 的大量链接实际来自最终 Tera 模板，不经过这个 Item 链接解析器，因此不能拿这个潜在问题解释当前 XIV 的大头；需要专门完整链接负载验证。

### 10.3 并行为什么排在后面

当前单线程可避免的树复制已很大，并行它首先会放大分配压力。Vault 也不是直接可共享的只读对象：ResourceSession 的 RefCell 状态、首次输入缓存、请求计费、图／环境缓存、求值特化与扩展 hook 都需要明确并行语义。

如果后续并行，优先纯且独立的已准备文档任务／模板任务，统一排序合并诊断和产物，保持资源预算与能力边界；不要简单给整个 Vault 套锁。发布并发写和旧目录删除则要另测文件系统竞争。

已有数据固定 CPU 8，不能外推“上 16 核即可提速 16 倍”；并行效果需要新的同输入基准。

## 十一、建议执行顺序与验收

### P0：优先处理共享内核的确定重复工作

1. bind／present／capture 不先深拷贝马上覆盖的 children／pending；内部呈现消费新拥有节点。
2. checked InstanceRecord／body 共享，运行状态与新增特化独立；原生目标避免不需要的 Code Scope／children 复制。

它们覆盖全部四个正文／模块负载；不要一开始重写整套 IR 或给所有节点套 Arc。

### P1：改善宿主批处理与确定的文件操作冗余

3. SSG 保留文档的 source／Environment 句柄，复用本轮配置解析和安全可共享的模块装配。
4. 发布目录去重创建；准备阶段复用已验证路由；内部源码使用共享 UTF-8 文本。

其中发布目录去重改动面很小，可与求值改造独立验证。文档句柄涉及 API 关联，不能用仅路径缓存替代正确语义。

### P2：需要额外探针或设计的项目

5. 参数槽位／实例 ID、输出路径预规划、按输出的链接索引。
6. 跨文档 checked 实例缓存、产物 spool、增量发布、可控并行。

按目标分别衡量：XIV 要减少宿主固定成本与文件操作，模块站要减少节点复制和调用装配。两者都是目标时不能只挑一个更容易改善的负载。

### 成本预算不是收益承诺

- XIV 的环境后续反查约占同批全程 2.68%；句柄方案值得做，但不能仅靠省反查声称消除整个文档分析阶段。
- XIV 配置解析约占 0.68%，Tera render 约占 7.80%；单独优化它们与优化整个准备／逐页阶段的收益上限不同。
- 若只是把发布阶段加速一倍，按本批其他阶段不变计算，XIV 全程约加速 1.12 倍，而不是两倍。若宽导出求值加速一倍，则对应全程约 1.55 倍。它们是 Amdahl 假设算例，不是已测或承诺的优化结果。
- 各项候选可能作用于同一段分配、复制和回收，不能把每项潜在收益简单相加；独立补充探针的 clone 时间更不能直接从 detail／普通 CLI 总计扣除。

### 验收不应只看 wall

- 五种完整负载、全部正文与相同输入；普通 release 的前后时间同批交错测量，探针单列，不拿不同批次秒数作收益。
- 输出路径、字节长度和逐文件 hash 相同；模块工作次数、Data 正向控制、依赖和来源仍可核对。
- 新增或保留重复挂载、未选分支、内容预算、循环累计、扩展映射、Analysis 不变、相对资源／来源和 `NeedResources` 的回归。
- 宿主覆盖同路径不同 source snapshot、最近配置、私有模块／私有数据、生成页面先登记再读取，以及失败扩展不安装输出。
- 发布覆盖空输出、已有输出、残留文件清理、目录／文件冲突、symlink、写入或 rename 失败与恢复；首次构建和重建分开报告。
- 对真正可疑但尚无单独时间的项，只加隔离探针，不预先承诺节省范围；必要时记录分配次数／复制节点数，不用有污染的旧 RSS 数据做论证。

## 产物

源码引用均指本报告开头的提交。本报告为新分析文档，先前 profile 报告与计时证据未改写。

- 详细计时和原始清单：`/tmp/opencode/notist-current-profile-20261103/`。
- 本轮派生的文件／目录数量、样例正文表格行数、模块调用与求值统计：`/tmp/opencode/notist-architecture-analysis-20261103/structural-evidence.json`。
- 本轮未实施上述优化，也没有提交／推送新改动。
