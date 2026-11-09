# 当前提交的整站热点复核

## 结论

当前工作负载的第一热点是求值中的内容物化／native 调用及相关短命对象构造、复制、销毁；
第二组是宿主路径键比较、正文遍历和 HTML 输出。不是“名字还没全部整数化”。

建议下一次先做一个减少无用构造的最小实验，而不是继续增加 ID、旁路缓存或全局名称池。
本轮没有修改生产实现，没有实施这些优化，也没有自动提交。

## 测量范围与方法

- 引擎为提交 01405db，正常源码由 git archive 重新导出，所有版本分别隔离 Cargo target。
- 使用相同的冻结完整站点：51,739 页，78,004 项输出、451,812,631 字节；不改 consumer。
- 证据目录：`/tmp/opencode/notist-hotspots-20261109`。
- 普通 release：一次预热、五次测量。另有两个仅启用 frame pointer 的 release 控制样本。
- coarse 三次：阶段计时，较少探针。detail 两次：递归调用和细分操作计时／计数。
- 两次用户态 CPU 采样，499 Hz；一次分配 API 概率采样。clean、计时和采样相互分开。
- 各运行均核对完整输出；16 次完整构建全部一致。

本机 perf_event_paranoid 为 2，samply 拒绝非 root 内核采样；未修改系统权限，也未启动／
重启采样服务器。改用 gperftools 的用户态 CPU timer，guard 只对指定的 Rust CLI
可执行文件启动 ProfilerStart，不采样扩展子进程。普通 CLI 仍没有探针或 allocator 替换。

CPU 使用 frame-pointer 诊断版本，保留原始 .prof、pprof 文本和调用栈。主要统计从原始
profile 自行解码并通过 addr2line 展开 inline frame；两次共 8,033 个样本，全部包含
目标 CLI 的调用帧。保留旧 pprof 的 Perl 警告日志，但主统计不依赖其文本解析。

CPU 百分比是 Rust 父进程的统计样本比例，不是包括扩展子进程的整站 wall 比例。
inclusive 项有包含关系，不能相加；self 项只计最内层执行位置。深层递归样本在统计
某个函数的 inclusive 时每条栈只计一次，避免因递归重复放大。

## 普通构建与探针扰动

| 版本 | wall 中位数 | user CPU | system CPU |
| --- | ---: | ---: | ---: |
| 普通 release，五次 | 8.1638 s | 7.4339 s | 0.9939 s |
| frame pointer 控制，两次 | 8.2198 s | 7.5078 s | 0.9634 s |
| coarse，三次 | 8.1404 s | 7.4494 s | 0.9361 s |
| detail，两次 | 10.4135 s | 9.6970 s | 0.9480 s |

普通五次 wall 为 8.146～8.204 s。首次预热为 13.185 s，单独保留、不纳入中位数，
也不称为严格冷缓存实验。已有相同输出目录，发布阶段主要是比较是否变化，不代表
首次写出完整站点的成本，更不能外推增量编辑或另一工作负载。

frame pointer 控制样本比普通中位数约高 0.7%，说明有扰动；coarse 差异较小但不因此
宣称没有开销。detail 明显慢约 28%，所以它用于调用次数和路径证据，不用于给真实
热点分配精确秒数。递归 present 的 inclusive 总量约 41.8 s，不是单次构建用了 41.8 s。

## 过程热点：时间主要花在哪里

下面是 coarse 三次中位数，比例相对于其 cli.total 约 8.077 s，不与普通样本混算：

| 过程 | wall | 约占总时间 |
| --- | ---: | ---: |
| 文档求值 | 2.994 s | 37.1% |
| 正文链接处理／HTML／该阶段回收 | 1.875 s | 23.2% |
| 文档分析，含宿主环境装配 | 0.929 s | 11.5% |
| 资源准备、复制及模板加载 | 0.775 s | 9.6% |
| 发布 | 0.436 s | 5.4% |
| 站点扩展与路由阶段 | 0.297 s | 3.7% |
| 公共模板上下文 | 0.137 s | 1.7% |
| 最终页面模板渲染 | 0.067 s | 0.8% |

求值与分析合计所在的 pipeline 阶段为 4.122 s，其中另有约 0.199 s 装配工作。
表中没有列全所有析构、装配、目录与返回边界开销；各项中位数也不保证严格相加。

detail 中 HTML 调用本身约 0.647 s，HTML registry 选择约 0.124 s；其余正文阶段还
包含链接遍历、路径解析、查表和文档／临时对象销毁，不能把剩余差值都叫做链接解析。
扩展进程阶段约 0.199 s，包括子进程执行和等待，不能等同于纯 I/O 等待。

user / system 时间可能累加多个线程和子进程，不能用 wall − user − system 推算 I/O。
本轮没有建立精确的 off-CPU 等待分解；阶段和 CPU 样本支持“求值是主要计算热点”，
不支持“每一次 stat/read 的耗时已经精确分开”。

## 操作热点：CPU 在做什么

两次采样的范围：

| 操作／调用路径 | CPU 样本占比 | 含义 |
| --- | ---: | --- |
| glibc malloc／free／realloc 等自身 | 26.9～27.0% self | 分配器的实际执行，不含全部 Rust wrapper |
| 完整文档求值 run | 36.9～37.8% inclusive | 与阶段计时的主要热点一致 |
| present | 35.6～36.0% inclusive | 包含 native execute、表达式、物化，不可重复相加 |
| execute | 25.4～25.6% inclusive | 调用实参、字段、派发、构造与其分配 |
| expr | 13.0～13.3% inclusive | 是 execute／present 内部的部分工作 |
| std::path::compare_components | 10.2～10.5% inclusive | 路径树键与规范路径比较，不是变量名比较 |
| memcpy／memmove 自身 | 2.5～2.9% self | 不涵盖全部复制与引用计数 |
| memcmp 自身 | 4.5～4.9% self | 与路径比较等有重叠，不能都计为名字查找 |
| HTML render_with_diagnostics | 7.2～7.8% inclusive | 与正文阶段 HTML 子项一致 |
| Node::descendants | 3.8～3.9% inclusive | 包括遍历栈分配与增长 |
| Dict::insert，各实参形式 | 约 3.0% inclusive | 字段键构造是其中一部分，不是全部成本 |
| 实例 implementation 查找路径 | 2.4～2.5% inclusive | 含 key 和快速缓存等子项 |
| 实例 key／编译期值 key | 1.1～1.3% inclusive | 结构化特化键目前不是首要优化方向 |
| 来源引用校验 | 1.0～1.3% inclusive | 不值得再以省略检查换取收益 |

约 19% 样本的调用栈还包含实际的 drop_glue／drop_in_place／Arc／Vec 析构帧；它与
free 等分配器样本重叠，不是额外可加的 19%，也不全是函数末尾的一次大析构。
例如临时 Runtime、Content、Piece 和字段字典会在求值过程中反复构造和销毁。

路径比较散落于 Vault::ensure_environment、Assets::insert、DocumentIndex::find、
Files::inspect、ResourceSession::register_source 和 publication::unchanged 等调用者。
它不能全部归给正文链接，更不能认为给编译器字段名加 Symbol 就能解决这 10%。

## 调用次数与分配证据

detail 对相同完整站点记录的调用次数：

- execute：6,073,286 次。
- implementation：6,073,286 次。
- present：8,440,426 次；递归计数，不是不同内容对象数。
- bind_content：2,213,972 次。
- data.native：1,955,784 次。
- 实际 data.parse：565 次；parse_hit：51,173 次。
- 实际模块编译：1 次；实际 bind_cached：3 次。

真实数据解析和模块绑定已有有效复用；继续围绕它们增加缓存，目前不是主要方向。
native／内容通路反复执行的是百万级操作，即便每次只做少量空对象构造也会累计成热点。

独立分配 probe 精确记录父进程 API 调用：malloc 132,729,790、calloc 37、realloc
14,829,086。它不包含 Python 子进程的分配；没有替换 System allocator。123,023 个
概率分配栈全部找到应用调用帧，记录无丢失。malloc／calloc 采样率为 1/2048，realloc
为 1/256，以下按逆概率估计事件数，不能当作逐调用点的精确计数：

| 最近应用分配点 | malloc 次数估计 |
| --- | ---: |
| Shared<Vec<Node>>::new | 约 1,658 万 |
| Dict::default | 约 1,085 万 |
| Shared<Vec<Piece>>::new | 约 1,040 万 |
| Dict::insert<&String> | 约 977 万 |
| Text::from(String) | 约 704 万 |
| Shared<BTreeMap<BindingId, Runtime>>::new | 约 620 万 |
| present | 约 603 万 |
| Node::descendants | 约 449 万 |
| Shared<ResolvedInstance>::new | 约 398 万 |
| Content::one | 约 394 万 |

notist::resources::normalize 另估计约 279 万次 realloc；这里是整个路径构造／增长通路，不说明
每次都发生物理复制。稀有大分配的字节估计方差大，本报告不以采样累计字节排列优先级。

这说明比起继续压缩 ID 字段，更值得检查“每次操作是否都需要创建这些短命表示”。
它并不意味着全部 27% allocator CPU 都能移除，也不意味着能缓存重复内容挂载的结果。

## 对应源码与更简单的候选方向

### 1. 先检查 native／内容路径中的无用构造

crates/notist-eval/src/eval.rs 的 execute：

- 在 implementation 返回后总是 Scope::new，只有 code 分支才插入和使用输入。
  native 调用也会创建并销毁一个空的共享 BTreeMap。可评估把输入环境的构造和装配
  放回 Code 实现路径，而非为所有调用增加另一个空 Scope 缓存或特殊 ID。
- 构造内容时先 item_shell，再覆盖 node.content；item_shell 已经克隆字段／实例并
  创建空 children。有些调用点随即丢弃这些中间内容。可评估直接一次构造带最终
  payload 的节点，保留同一份 attrs、origin、level、span 复制逻辑。
- Dict::insert 先 Into<String>，新键再转 Text；传入 &String／&str 时有重复构造机会。
  更自然的方向是只在新增时创建最终键，更新复用旧键，而不是先引入 FieldId。

这些只是有观测证据的候选，尚未量化优化收益或完成实现验收。不能为了移动代码而
改变实参先求值、特化错误顺序、调用深度、数据／内容预算、来源与重复挂载语义。

### 2. 宿主路径与索引的重复工作

路径比较约占一成 CPU，且横跨环境、资源、链接、发布多个阶段。应先检查同一输入
视图内是否重复规范化／拼接／构造 PathBuf 和重复查表，再决定现有表示能否直接收敛。
不能直接跳到全局 ResourceId 或缓存授权结果；根目录、修订、相对基准与错误顺序仍要
保持原语义。已有产物比较约 0.384 s，值得单独观察，不以跳过内容核对作为优化。

### 3. 减少重复树遍历与遍历栈分配

Node::descendants 每次都先 vec![self]，随后 extend children。该通路既有 CPU 证据，
又有大量 malloc／realloc 证据。HTML、组件收集、链接与嵌入检查可能反复访问同一树。
应先寻找能合并的遍历职责或能自然复用的工作空间，不引入无界递归或改变先后顺序，
也不为了少遍历漏掉来源、链接或构造检查。

### 4. 暂缓扩大 ID 工程

实例字符串键仍有约 1.2% CPU，可作为后续结构化表示实验，但不是主热点；不能把
implementation 的约 2.4% 和其中的 key 再相加。Symbol／FieldId 无法解决上述大部分
容器、物化和路径成本。当前先减少不必要的构造，比新增身份域或旁路缓存更符合
“实现本身变简单”的要求。

## 复现与限制

工具：tools/profile_hotspots.py、tools/analyze_hotspots.py、tools/hotspot_cpu_guard.c。
所有源副本、二进制、原始采样、inline 符号缓存、阶段日志、分配栈、汇总与工具副本均
保留在证据目录。采样 guard 的目标限制、实际 profile 解析、递归去重和表示匹配均有
工具回归测试。

这是当前冻结完整构建工作负载的热点图，不是增量编辑、worker 交互或不同站点的
热点保证。没有实现优化，所以不声称已经获得任何加速或降低内存。
