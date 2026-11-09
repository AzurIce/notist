# 求值执行与内容物化重构验收

## 范围与结论

基线为 b60e8a8，候选代码提交为 76e7563，位于 refactor/eval-execution。checked IR、compiler、wire、直接 embedding、native/data/Code 参数消费与内容发射已成组迁移；不保留第二套解释器或名字回退通路。SSG、Book、HTML、资源授权与身份命名空间未修改。

- Invocation 受控构造与编辑，在声明位置绑定参数，保留 supplied order、重复条目与额外字段。
- 临时实参归调用所有；native 按角色借用或消费，Code 按 BindingId 槽位接收。只有 constructor 冻结持久字段 DictBuilder。
- 唯一 invoke 返回 CallOutcome；立即发射的 ElementDraft 不包装单元素 Content，最终 children 完成后一次构造 Node。
- Content 使用 Empty/One/Many，共享存储持有逻辑节点计数；空内容零分配，singleton 直接共享，无临时 Vec。Ready 复用具体 payload/children，来源元数据单独持有。
- 发射追加到最终输出；失败仅回滚相应后缀，保留诊断、资源请求和累计预算。森林仍向每个 piece 传入相同原始限额，循环使用剩余额度。

## 语义与结构

默认整工作区测试 878 项通过，全特性 888 项通过，无忽略项；编译 jobs=2，test-threads=2。新增实参顺序、optional 捕获重放、重复/额外参数、声明编辑重绑定、非法公开位置查询和异常 embedding 布局诊断测试。

外部程序独立链接两侧生产 rlib，比较 58 个求值场景的完整 root、fields、attrs、instance、origin、diagnostics、dependencies、sources 或 Pending 路径；再比较任意顺序、重复/额外条目的 wire 往返，共 59 项全部一致。没有归一化身份或来源。

结构断言通过：空 Content 构造/clone/drop 零分配；singleton 一次分配；绑定 singleton Ready 无临时 Vec；固定标量实参准备零分配；builder 保持共享键、重复更新、宽字典查询、COW 与逻辑预算。正常输出无 Expr，输入和捕获内容不被改写。

外部异常记录的显式契约：选定声明与实现的 kind/参数位置布局必须一致；不一致时在实参求值后诊断失败，不保留偶然的名字回退行为。不承诺畸形外部 IR 的旧错误文本不变。Rust API 说明位于 crates/notist-eval/README.md，语言与文本 wire 不变。

## 独立求值 A/B

每侧三个新进程，每进程七轮 shuffled 样本，40ms 目标批次。普通 release 计时包括求值和结果析构，不含编译、夹具准备、HTML、文件 IO 或校验；分配统计来自单独 alloc-metrics 构建。每轮在计时外校验完整结果。

以下为三个进程中位数的中位数，尺寸均为 128；请求 bytes 是累计分配/重分配流量，不是 RSS。

| 场景 | 基线 µs | 候选 µs | 时间变化 | 分配次数 基线→候选 | 请求 bytes 基线→候选 |
|---|---:|---:|---:|---:|---:|
| native_text | 49.096 | 37.173 | -24.3% | 1297→527 | 116104→66944 |
| code_calls | 94.944 | 74.642 | -21.4% | 2847→1177 | 320212→195748 |
| many_parameters | 37.606 | 10.911 | -71.0% | 210→46 | 58572→30444 |
| capture_uses | 52.195 | 40.002 | -23.4% | 1312→535 | 131020→80964 |
| recover_errors | 73.080 | 62.775 | -14.1% | 2845→1815 | 196204→136252 |
| strict_lets | 8.258 | 8.088 | -2.1% | 60→47 | 21708→21052 |
| json_cached | 232.700 | 175.422 | -24.6% | 8114→3116 | 615559→309847 |

不声称每个控制项都更快：ready_text/1 为 0.341→0.358µs，约 +17ns/+5.0%；ready_text/16 约 +0.3%，ready_text/128 约 +1.9%。这些 Ready 对照项的分配次数从 6 降到 5，请求流量各少 8 bytes；小规模控制项仍有固定分派代价。nested_expansion/16 为 7.351→7.323µs。全部 58 行、进程中位数和原始样本保留在外部 evidence，不剔除不利数据。

## 整站五组交错 release A/B

冻结的同一份输入，完整发布包含 78,004 个文件、451,812,631 bytes。每次构建后在计时外核对全部路径、长度和 SHA-256，十次全部一致；六次独立 CPU 采样构建也核对相同完整产物。输入清单与源码/二进制摘要检查通过。计时不与本任务编译或测试并行；OS cache、CPU 频率及其他系统负载未控制。

| 组 | 基线秒 | 候选秒 | 基线峰值 RSS KiB | 候选峰值 RSS KiB |
|---|---:|---:|---:|---:|
| 1 | 13.284 | 7.120 | 2829560 | 2757152 |
| 2 | 7.743 | 7.339 | 2830284 | 2756444 |
| 3 | 7.799 | 7.155 | 2829720 | 2756340 |
| 4 | 7.745 | 7.151 | 2830260 | 2757252 |
| 5 | 7.762 | 7.211 | 2830728 | 2756616 |

不删除首组慢样本。构建中位数 7.762→7.155s，约 -7.8%；进程峰值 RSS 中位数 2830260→2756616 KiB，约 -2.6%。RSS 来自 GNU time，不是请求流量或 evaluator 独占堆。

## 布局与持有内存

| 公共 checked 类型 | 基线 sizeof | 候选 sizeof |
|---|---:|---:|
| Invocation | 144 | 144 |
| InstanceRecord | 312 | 336 |
| TypedExpr | 136 | 136 |
| FunctionDef | 120 | 120 |
| ParameterDef | 160 | 152 |

候选内部布局：Content=16、Piece=152、Runtime=48、Construction=72、ArgumentValues=136 bytes。逻辑计数在共享存储，不为每个 Runtime 槽位增加占用。

分配诊断另外记录 prepared 夹具的持有堆、峰值、output-live 和析构后状态。prepared 测量包含源文本、checked IR、Context、资源与 canonical 快照，不冒称纯 IR 堆大小；静态大小变化不能只凭 runtime 分配下降推断。全部求值场景的结果析构后额外请求存活量，两侧均为 0。完整请求峰值与持有量在 allocation/prepared JSON。

## 求值范围内 CPU 归因

内核拒绝 perf event 采样，未修改系统权限。使用 gperftools 499Hz 用户 CPU timer、libunwind DWARF 和原始 release 二进制，仅采选定父进程，不采 Python 子进程。每侧三次整站构建；基线求值范围 3972 个样本，候选 3278 个样本。

按最近语义职责互斥归类，不把方法搬家当作热点消失：候选 ArgumentValues 与 invoke 合并，对照基线 execute 的调用装配。下列是每次构建的近似采样 CPU 秒，不是普通 A/B wall time。

| 互斥职责 | 基线 | 候选 |
|---|---:|---:|
| 调用装配、实参消费及持久字段 | 0.996 | 0.784 |
| 发射与挂载 | 0.724 | 0.451 |
| 实现解析 | 0.191 | 0.178 |
| 表达式执行 | 0.168 | 0.221 |
| 内容绑定 | 0.234 | 0.216 |
| native data 与资源 | 0.212 | 0.198 |
| run 校验与清理 | 0.128 | 0.143 |

另一互斥叶操作视图：allocator 约 0.784→0.559s，drop/引用计数约 0.362→0.132s，clone 约 0.283→0.203s，memory copy 约 0.060→0.113s。两种视图不能相加；内联、采样误差和职责移动使单项变化不等于因果证明。

## 证据与复现

完整 evidence 位于 /tmp/opencode/notist-execution-refactor-b60e8a8/：冻结基线源码、独立普通/分配 target、环境信息、源码及二进制 SHA-256、输入/完整输出清单、default/all-features 测试日志、三进程逐样本 JSON、五组整站日志、完整结果差分、prepared 堆探针、原始 CPU profile 与 inline 符号映射，以及 validate.py、extra-checks.py、cpu.py、report.py、finalize.py。先前布局测量另存 first-pass 与 layout-pass，不充作最终候选结果。

普通回归：`CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 cargo test --workspace --locked --offline -j2 -- --test-threads=2`；全特性增加 `--all-features`。求值 driver 用 tools/eval-bench，普通计时与 `--features alloc-metrics` 构建分开。完整站点须使用 evidence 内同一冻结夹具与清单，不以减少页面或绕开发布检查替代。
