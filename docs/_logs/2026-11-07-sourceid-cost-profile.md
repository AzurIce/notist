# SourceId 完整成本归因：CPU、分配、内存与诊断对照

## 结论

**主要回退不是 32 位 ID 或可回收弱表的必然代价，而是我新增的来源校验器存在
双重遍历入口，放大了对 checked IR 的扫描。** 这应当作为实现问题处理，不能用
“完整生命周期总要付出成本”来笼统解释。

独立诊断版本只移除这个重复入口，保留所有来源校验类别：

- 来源查询从 208,134,817 次降到 20,967,782 次，约减少 89.9%。
- 原版/去重/完全不校验的四轮诊断对照，wall 中位数分别为 9.744 / 9.341 / 9.306 s。
  去重拿回了完全跳过校验所能省掉时间的大约 92%；后者只是不可接受的成本上界探针。
- 另做四对旧基线/去重版本交错复测：9.614 → 9.468 s（约 −1.52%），user CPU
  8.712 → 8.613 s（约 −1.14%）。这是未合并的诊断候选证据，不是生产实现已加速。
- 新增的 257,668 次 realloc 与校验遍历栈的精确计数完全吻合。普通分配实际上减少
  156,978 次；将 realloc 也算入“分配事件”，净结果才是多 100,690 次。
- 内存收益是真实的：本轮 clean 峰值 RSS 降 39.12 MiB，独立 Rust 请求堆峰值降
  52.61 MiB。没有发现 51,739 个页面来源被模块缓存保活；最后只剩一个模块身份。

生产 Rust/Cargo 实现未修改；去重和不校验版本只存在外部冻结目录。没有提交、推送、
改 consumer、改依赖锁或改默认预算，也未推进 DefId。

## 1. 输入、版本与测量隔离

证据目录：`/tmp/opencode/notist-source-cost-20261107`。

before/after 直接复制上一轮验收的冻结源码和无插桩二进制，并校验 SHA-256。
before 为 `a168131`；after 为完整 SourceId 未提交实现，不是永久池原型。
正式 A/B 的两个二进制没有重新插桩或更换分配器。

完整输入为 53,102 个文件、711,605,507 字节，包含 51,739 个页面；每次完整构建
结束后，在计时之外逐一校验 78,004 项产物、451,812,631 字节。
本轮共 56 次成功的完整构建：初次发布 1、clean 16、CPU 4、基础 PMU 4、cache/TLB
PMU 4、native 分配栈 4、Rust ledger 3、三版本对照 12、旧基线/去重对照 8。

所有诊断与 clean 时间分开。源码探针及反事实版本均在 `sources/` 下独立副本中，
保存源码、二进制、输入及产物哈希。首次失败的 perf 默认元数据录制保留在
`failed-default-perf-metadata/`，不算成功样本、不参与性能统计。

物理冻结目录的名字会改变内部路径字符串长度，可能改变格式化字符串的容量增长；
因此本报告只计算本轮同目录成对差值，不拼接上一轮的绝对分配次数。

## 2. clean：回退可复现，但不是固定的 4.71%

八对交错完整构建，首轮 I/O 异常样本照常保留，没有挑选删除。

| 中位数 | before | after |
| --- | ---: | ---: |
| wall | 9.4549 s | 9.7504 s |
| user CPU | 8.6553 s | 8.9499 s |
| system CPU | 1.0397 s | 1.0440 s |
| 峰值 RSS | 3,360,494 KiB | 3,320,434 KiB |
| minor faults | 885,376.5 | 874,851.5 |

总体 wall 中位数比值为 +3.13%，每对比值的中位数为 +2.99%。配对 bootstrap
描述性区间约为 +2.75%～+3.17%；同机连续样本不是独立机器重复实验，不能把它
当成可移植的严格置信保证。

第一对 before 为 11.955 s，有 13 个 major faults、20,953 次 voluntary context
switches；其 user CPU 仍为 8.674 s。该 wall 异常更符合 I/O/等待而非语言计算。
后续典型配对仍慢约 3%，且 user CPU 同向增加，因此不能把回退全部归为噪声。

## 3. CPU：perf 采集，samply/framehop 展栈

通过 Nix 使用 perf 和 samply，没有修改 `perf_event_paranoid=2` 或使用 sudo。
samply 直接录制的前置检查拒绝该设置；perf 默认请求的内核调用链/BPF 元数据也会
被拒绝。以下用户态模式实际可用：

```sh
perf record --no-kernel-callchains --user-callchains --no-bpf-event \
  -e cpu-clock:u -F 499 --call-graph dwarf,16384 -o cpu.perf.data -- ./notist build
samply import --save-only --unstable-presymbolicate -o cpu.json.gz cpu.perf.data
```

两对交错 CPU 采样，筛选 Rust 进程及其工作线程，排除 Python 扩展进程的 CPU。
before 有 8,505 个 Rust 样本，after 有 8,798 个。

- after 中 `validate_references` 占 489 个，即约 **5.56%** Rust CPU 样本。
- 其中 461 个经 `validate_program`，占校验样本约 **94.3%**：主要是每页重新遍历
  模块/模板 checked IR，而非主文档的简单来源检查。
- 登记 `in_namespace` 只有 11 个样本，身份析构相关有 28 个；不是几百毫秒回退的
  主要解释。独立探针对登记/析构两处约 10.35 万次锁获取的累计计时约 2.22 ms，
  也不支持“锁竞争是主因”。
- 全站原有分配器、Arc、路径/BTreeMap 仍是重要热点。SourceId 没有消除所有路径
  比较或所有引用计数，不能拿其它路径热点的比例作为本项预期加速幅度。

### 栈质量与符号化

本机 `perf script/report` 的 libdw 展开遗漏了大量 Rust 栈，不能用它的空栈结果
进行归因。**最终统计与交互文件使用 samply/framehop 从原始寄存器/用户栈展开，
再由本机 addr2line 补充函数、行号和 inline frame。** 在 Firefox Profiler 中实际
打开并验证了完整调用树和 `validate_references` 过滤火焰图。

保留原始 perf.data、原始 samply JSON 及重新符号化的 `*.symbolicated.json.gz`。
第一份 before 录制报告丢失 29 个样本，其余三份为 0，原始日志保留；比例及展开
方法的限制不能忽略，因此百分比视为统计估计，不把采样与 scope 插桩耗时逐项相加。

分配栈初版派生分析还修正了一个 lld PIE 符号化问题：PT_LOAD 的虚拟地址不一定
等于文件偏移，不能按 text 映射的 `start-offset` 直接重定位。最终按 offset=0
映射的加载基址解算，加入了回归测试；错误派生结果另存，不用于本报告。

### PMU 独立计数

| 用户态计数中位数 | before | after | 观察 |
| --- | ---: | ---: | --- |
| cycles | 48.212 B | 49.627 B | +2.93% |
| instructions | 131.671 B | 144.877 B | +10.03% |
| branches | 28.404 B | 30.886 B | +8.74% |
| branch misses | 75.709 M | 80.823 M | +6.75% |
| cache references | 2.580 B | 2.782 B | +7.85% |
| cache misses | 113.192 M | 117.713 M | +3.99% |
| dTLB load misses | 17.385 M | 17.081 M | −1.75% |

B/M 表示十亿/百万。计数包含 perf inherit 的子进程，不与仅 Rust 的 CPU 样本
混为一个分母。PMU 事件含义依本机 CPU；`cache-*` 不是通用的 L1 命中率。
软件 context-switch 的 `:u` 值不能代替实际调度计数，调度分析采用 wait4 的 rusage。
额外的大量指令/分支与重复遍历一致，不是单纯 wall 等待；这仍不是逐指令因果分账。

## 4. 根因：把内容投影和存储 IR 当成两棵树一起走

当前 `Sources::validate_references` 的 Node 分支既做：

```rust
pending.extend(node.children().iter().map(Reference::Node));
if let Some(expression) = node.as_expr() {
    pending.push(Reference::Expression(expression));
}
```

但 `Node::children()` 对表达式节点可以返回其 literal content children；沿着
`ExprKind::Call.children → ExprKind::Content` 又会走到相同节点。不是两个独立的
来源结构，而是同一个结构的两种视图。

对于连续包含这类节点的深度 d，存在 `T(d)≈2T(d−1)` 的重复路径风险。并非声称
所有文档都是指数复杂度，但这里确实会在较深的字面量嵌套上放大工作；且来源
预检发生在语言执行预算之前，不能指望执行步数预算截断它。

诊断去重版本仅将上述入口改成互斥分支：

```rust
if let Some(expression) = node.as_expr() {
    pending.push(Reference::Expression(expression));
} else {
    pending.extend(node.children().iter().map(Reference::Node));
}
```

Node 自身 origin/expansion 仍检查，表达式 source、调用 source、参数及全部分支
仍从 IR 检查；没有按 ID 直接跳过，也没有取消版本冲突或 missing-source 检查。
它不是任意 DAG 的全局去重器，不声称每个共享对象永远只访问一次。

| 精确工作量 | 当前 | 移除双入口 |
| --- | ---: | ---: |
| 来源查询 | 208,134,817 | 20,967,782 |
| Node 访问 | 50,841,561 | 3,868,591 |
| Expr 访问 | 105,111,651 | 11,474,087 |
| walk 次数 | 155,216 | 155,216 |
| walk 初始分配 | 155,216 | 155,216 |
| walk realloc | 257,668 | 205,930 |

总访问量减少约 90.2%，但初始分配没有减少，realloc 也只减少 51,738 次。
因此主要 CPU 收益来自不再重扫大量节点/查表，不仅仅是少调用了 realloc。

### 受控诊断对照

| 四轮交错，均无计时/分配探针 | wall 中位数 | user CPU |
| --- | ---: | ---: |
| 当前 after | 9.7443 s | 8.9343 s |
| 移除双入口 | 9.3409 s | 8.5516 s |
| 完全不做来源 walk | 9.3063 s | 8.5204 s |

完全不校验版本故意不满足 API 契约，仅估计这组输入的校验成本上界，绝不能作为
可合并优化。不能把三个不同二进制中位数相减当成精确的可加成本模型。

另用四对旧基线/移除双入口的独立交错对照得到 9.6138 → 9.4680 s。去重副本还
通过了 5 项 `source_identity` 所有权/wire/快照/扩展回滚回归测试，并非只靠首页
看起来相同。生产实现尚未采用该改动。

## 5. 分配：增加的是增长事件，非普通 malloc

独立 Rust GlobalAlloc ledger 记录请求布局，并按调用 scope 归类，不更换 System
分配器。另做两对原生 glibc API 计数/概率调用栈采样交叉验证；后者 malloc/calloc
按约 1/2048、realloc 按约 1/256 抽样，不拦截 Python 进程计数。

| 精确 Rust 请求事件 | before | after | 差值 |
| --- | ---: | ---: | ---: |
| alloc + alloc_zeroed | 181,494,421 | 181,337,443 | −156,978 |
| realloc | 14,570,871 | 14,828,539 | +257,668 |
| 合计 | 196,065,292 | 196,165,982 | +100,690 |

native 两对样本的 realloc 差值也都是 257,668，普通分配差值相差至多几个启动事件。
新增 realloc **100% 对应 validate_walk**，不是猜测注册表或缓存“可能多分配”。

来源 scope 的精确分账（exclusive 分配事件，不把父子 scope 重复相加）：

| 部分 | 新增 alloc | 新增 realloc | 请求字节 |
| --- | ---: | ---: | ---: |
| register | 155,235 | 0 | 14,897,492 |
| insert/COW | 206,950 | 0 | 19,039,504 |
| validate_walk | 155,216 | 257,668 | 2,483,456 + 105,066,336 |

同时，其它 scope 的普通分配净减少 674,379 次；这是其它部分的净变化，不能全部
直接命名为“消除的路径分配”。Scope 外也包含 namespace 参数创建等新成本。

校验临时栈总计产生约 102.57 MiB 请求流量，但全部及时释放，并不是峰值常驻堆。
这一版用 `vec![root]` 开始，工作表从容量 1 多次增长。去重后仍有 205,930 次增长，
说明后续有优化 scratch 复用/容量策略的空间，但不是第一优先级，也不宜盲目给
每个 scope 预分配很大的向量。

剩余分配画像仍大量来自数据/字典/Scope、资源路径处理、格式化、内容构造等。
采样栈只用于查调用位置，估计字节会受罕见大对象影响；0.05% 的全局差值远小于
粗抽样自身误差，不能靠两次抽样估计值相减证明它。以精确总计/ledger 为准。
泛型/ICF 可能给 RawVec 机器码选出另一个类型名，不能把 `Vec<Diagnostic>` 的
折叠符号当成实际产生了百万条诊断；派生表优先归到外层应用函数。

## 6. 内存：紧凑表示有效，回收没有变成永久池

| 本机 sizeof（字节，非稳定 ABI） | before | after |
| --- | ---: | ---: |
| Node | 136 | 128 |
| Origin | 32 | 24 |
| SourceLocation | 24 | 12 |
| TypedExpr | 160 | 136 |
| Invocation | 208 | 184 |
| Diagnostic | 64 | 48 |

before 的布局沿用归档的旧基线布局二进制，并逐一确认其 24 个 core Rust 源文件与
本轮 before 完全相同；未使用被回退原型的 after 布局。after 对当前同字段布局的
诊断 core rlib 实测。编译命令、二进制及来源核对见 `layouts.json`。

| 精确 Rust requested-live checkpoint | before | after |
| --- | ---: | ---: |
| pipeline 后 | 2,351,223,753 | 2,296,055,187 |
| bodies 前 | 2,706,452,780 | 2,651,284,214 |
| bodies 后 | 962,114,175 | 965,134,818 |
| 返回 build artifact 前 | 1,404,814,317 | 1,407,834,960 |
| CLI 局部对象析构后 | 266,748 | 2,953,498 |
| 峰值 | 2,747,028,885 | 2,691,860,231 |

请求堆峰值下降 55,168,654 B，约 52.61 MiB；clean RSS 下降约 39.12 MiB。
两种指标不同，glibc 可用容量/空洞/缓存和映射页不会完全等于 Rust 请求布局，
不能把 requested-live 当 RSS，也不能把 probe 的 wall 当 clean wall。

after 创建 51,740 个命名身份，退休 51,739 个；唯一留下的身份与模块缓存保活
相符，页面身份已退役。尾端反而多约 2.56 MiB，主要符合弱 HashMap 保留高水位
bucket 容量：它删除了键和值，但没有每次 shrink 容量。这不是“仍持有全部页面
来源名/正文”的证据。它可以在维护点按阈值回收，但不应在每个析构时反复 shrink。

本次没有对长驻 LSP 连续数千轮编辑做完整压测，因此不外推为“所有服务路径均已
证明永不增长”。已有键回收、修订、并发及匿名身份单测仍保留。

## 7. 优先级与边界

1. **先修正双入口遍历**。改动小、没有删除校验类别，有独立完整 workload 和所有权
   回归证据。合并后应重跑完整 Rust/工具测试及 clean A/B，不能把诊断数据直接当验收。
2. 再评估工作栈复用/合理容量。它可减少事件数，但已不是主要 CPU 差距；记录峰值和
   逻辑预算，不用大容量常驻 scratch 换来虚假的“低分配”。
3. 模块 checked IR 按页重校验仍值得设计更便宜的所有权证明，但不能仅凭 SourceId
   或文件名缓存“校验通过”。必须绑定 immutable IR、对应源码修订及来源拥有关系。
4. COW 与 namespace 复用为次要项：本轮 103,474 次 insert COW、206,950 次相应分配；
   不能为了少复制而覆盖旧 revision。不要先扩大到 DefId、路径算法或缓存策略重写。
5. 保留弱注册表的回收、域/匿名区分、冲突拒绝及显式 wire；没有证据支持退回永久池。

CPU profile、硬件计数、分配栈与精确 scope 对上了相同原因。应先处理这个可避免的
实现问题，而不是把 +3% 接受为 SourceId 设计必然代价，也不是把内存收益当作性能
问题的替代解释。

## 8. 复现与交互文件

脚本：`tools/profile_source_cost.py`、`tools/analyze_source_cost.py`、
`tools/source_alloc_stack.c`、`tools/source_cost_probe.rs`、`tools/source_cost_layout.rs`。
源探针只注入外部副本；正常 before/after 二进制 SHA 与上次验收完全一致。

```sh
python3 tools/profile_source_cost.py prepare --out /tmp/opencode/source-cost-REPRO
python3 tools/profile_source_cost.py build-diagnostics --out /tmp/opencode/source-cost-REPRO
python3 tools/profile_source_cost.py clean --out /tmp/opencode/source-cost-REPRO
python3 tools/profile_source_cost.py cpu --out /tmp/opencode/source-cost-REPRO
python3 tools/profile_source_cost.py alloc --out /tmp/opencode/source-cost-REPRO
python3 tools/profile_source_cost.py cache-counters --out /tmp/opencode/source-cost-REPRO
python3 tools/profile_source_cost.py diagnostics --out /tmp/opencode/source-cost-REPRO
python3 tools/profile_source_cost.py walk-ledger --out /tmp/opencode/source-cost-REPRO
python3 tools/profile_source_cost.py safe-compare --out /tmp/opencode/source-cost-REPRO
python3 tools/analyze_source_cost.py --out /tmp/opencode/source-cost-REPRO
python3 tools/profile_source_cost.py verify --out /tmp/opencode/source-cost-REPRO
```

硬件计数需本机支持所选事件；脚本保存了本次实际使用的 Nix 工具路径/hash。
CPU 的 self/cumulative 汇总见 `cpu-summary.json`，精确分账见 `ledger.json` 和
`walk-once-ledger.json`，概率分配调用位置见 `alloc-attribution.json`。

交互查看（已经本机打开 after 并过滤 `validate_references`）：

```sh
nix run nixpkgs#samply -- load \
  /tmp/opencode/notist-source-cost-20261107/cpu-0-after.symbolicated.json.gz
```

before/after 各两份完整、已展开 inline frame 的交互 JSON 均保留；未点击 Share
发布 profile。当前本机查看服务端口为 3791，停止服务后仍可用上述命令重开。

最终 `integrity-check.json` 复核全部 56 次成功构建产物、所有冻结源和二进制，
并确认生产区 246 个既有 Rust/Cargo 文件与上次验收完全一致。新增分析工具通过
61 项 Python 测试；独立 allocator/scope 探针的 2 项 Rust 测试通过；去重副本的
5 项来源所有权测试通过。正常 before/after 仍是上次验收的同一 SHA 二进制。

## 2026-11-08 修复进展

上述“尚未落生产”描述的是本次归因结束时的状态。用户随后授权修复，互斥遍历
已应用到生产源码，并补充精确访问次数和缺失来源检查的三项回归测试。
本轮完整测试及正式 clean A/B 记录见同目录 `2026-11-08-sourceid-traversal-fix.md`。
