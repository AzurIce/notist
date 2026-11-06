# IceXIV DB 全量性能 profile（2026-11-04）

## 结论

**主要瓶颈是求值器及内容树生命周期，不是 JSON 解析、模块编译或 Tera。**

- 无插桩 release CLI：5 次中位数 **15.124s**，范围 **14.981–15.281s**。
- 同一个无插桩 serve 进程：首次 **14.489s**，随后三次同内容重建 **15.106 / 13.446 / 13.429s**。
- 保留原始 SSG 析构顺序的 coarse 对照，组装均值 **14.406s**：求值 **7.216s**，SSG + Vault 收尾销毁约 **3.090s**，正文/链接处理 **1.697s**。
- Rust CLI 进程独立 RSS 观测峰值 **6.54 GiB**；这不是 Python 启动器的 `ru_maxrss`，也不是分配字节数。

用户提供的 `✓ Updated · 51739 pages · 18222ms` 没有被稳定复现。本次没有实施性能优化，因此不能把 18.222s 与本次 13–15s 的差值称为优化收益。缓存、调度、频率、进程生命周期和整机负载均未完全控制。

原始证据：`/tmp/opencode/notist-icexiv-profile-20261104/`。

## 1. 版本与真实工作负载

- Notist：`85ab69e13bb99cb4016538accdd55c65f0cb6d7c`；与 icexiv-db 当时 `flake.lock` 的 Notist pin 一致。
- icexiv-db：提交 `9cf1646cac302a9a58740071bc1aac777746d668` **加开始测量时的工作区内容**，不是该提交的干净 checkout。保留未提交的游戏导出、分块 Data、图标及模板改动。
- Notist 测量期间出现另一批生产代码修改；本次不修改、不纳入这些进行中的改动。所有 Rust 插桩仅在结果目录的源码副本中。
- CPU：AMD Ryzen 9 9950X3D2 16-Core Processor，32 个逻辑 CPU。保留可用 CPU 0–31，保留生产模板策略：大站最多 8 个模板线程，不采用旧实验的单核绑核。
- Rust：具体 `rustc -Vv` 和系统版本保存在 `manifest.json`。release，`--locked`，默认功能；诊断二进制另启用 `profiling`，release debug symbols 为 1，关闭 incremental。

| 工作量 | 数量 / 逻辑字节 |
| --- | ---: |
| 物品详情 | 51,225 页 |
| 目录 | 513 页 |
| 首页 | 1 页 |
| HTML 合计 | **51,739 页** |
| `.data` JSON | 52 个物品块 + 513 个目录文件 = **565** |
| 静态输入 | 26,263 文件，333,712,731 B |
| 最终输出 | **78,004 文件，451,812,631 B** |

这里的字节是文件逻辑长度，不是 `du` 的磁盘分配量。没有限制页数、去掉图标、删字段、改预算、改依赖锁、改准备脚本或改 Data 模型。

## 2. `Updated` 到底测什么

基线 `crates/notist-cli/src/serve.rs::rebuild` 的边界是：

1. `Instant::now()`。
2. `target.kind.assemble(target)`：重新创建 Vault、生成全部页面、分析、求值、资源处理、HTML、模板、临时 SSG 对象及 Vault 释放。
3. 把 `started.elapsed()` 交给成功报告。
4. 再替换前一个内存站点，最后显示终端报告。

因此 **`Updated` 不包含磁盘 publish**，也不包含 200ms watcher debounce、之前站点替换时的释放、HTTP 传输或浏览器渲染。它也不是单页增量时间：同内容写回首页，仍然重做全站组装。

`notist build` 的进程 wall time 还包含磁盘发布、进程初始化/退出等工作，不能直接与 `Updated` 等同。当前 `assemble` 每轮新建 Vault；模块层的进程内 comemo 与同一批次的 Data 缓存，不等于跨重建保留整个站点的结果。

## 3. 无插桩计时与诊断开销

每种 CLI 模式先 warmup 一次。正常版测 5 次，coarse/detail 各测 3 次，顺序交替；每次是全新 CLI 进程。所有正式 CLI 样本都已有相同输出，publish 走完整内容比较，不是首次写盘。

| 模式 | 进程 wall 中位数 | 最小–最大 | user 中位数 | system 中位数 |
| --- | ---: | ---: | ---: | ---: |
| normal，无插桩 | **15.124s** | 14.981–15.281s | 13.489s | 1.753s |
| coarse，阶段诊断 | 15.169s | 15.024–15.434s | 13.612s | 1.819s |
| coarse，原析构顺序独立对照 | 15.035s | 14.904–15.143s | 13.545s | 1.748s |
| detail，密集调用树诊断 | 17.788s | 17.745–17.824s | 16.238s | 1.780s |

正常版五次 wall：`15.280815, 14.981082, 15.123709, 15.160346, 15.061903` 秒；按其中位数约 **3,421 页/s**。这只是整站平均吞吐，不是每页延迟分布。

第一次无输出的 normal warmup 为 **21.517s**，包含首次发布，且未经相同预热；仅作场景记录，不能混入上述正常重复构建分布。detail 比正常版中位数高约 **17.6%**，所以密集探针的秒数不能冒充正常运行耗时。

serve 是另一组无插桩测量，直接读取生产终端 `Updated` 数字：

| 同一 serve 进程 | Updated |
| --- | ---: |
| 初次组装 | 14.489s |
| 第一次同内容重建 | 15.106s |
| 第二次同内容重建 | 13.446s |
| 第三次同内容重建 | 13.429s |

只在副本的 `content/index.not` 写回完全相同的字节来触发 watcher。三次重建中位数 13.446s，但样本只有 3 个，不能据此宣称所有稳态重建都固定需要 13.4s。

## 4. 组装的互斥阶段分解

以下是 **保留原始 SSG 自动析构顺序的独立 coarse 对照三次均值**，以其 `cli.assemble = 14.406s` 为分母，不是把 detail 时间缩放到 normal，也没有把父阶段和子阶段相加。

| 互斥工作 | 秒 | 组装占比 |
| --- | ---: | ---: |
| 求值 `pipeline.evaluate` | **7.216** | **50.1%** |
| SSG + Vault 收尾销毁 | **3.090** | **21.5%** |
| 正文 HTML、链接、registry、页面元数据 | **1.697** | **11.8%** |
| 分析 `pipeline.analyze` | 0.952 | 6.6% |
| 静态资产准备与模板加载 | 0.776 | 5.4% |
| 站点扩展与生成路由准备 | 0.296 | 2.1% |
| 共享模板上下文 | 0.087 | 0.6% |
| 页面模板渲染与写入内存输出 | **0.070** | **0.5%** |
| 其余管线、宿主准备等 | 0.221 | 1.5% |
| 合计 | **14.406** | **100%** |

该对照的 CLI publish 平均 **0.397s**，其中完整现有输出比较 **0.359s**；主 coarse 分别为 0.438s / 0.400s。该部分不属于 `Updated`。

**析构顺序对照揭示了一个重要归因陷阱：** 主 coarse/detail 在成功返回边界显式计时部分局部变量，会改变某些相对销毁顺序；独立 ordered 对照移除七个显式 SSG Drop，保持原来的自动销毁顺序，其它工作不删减。

| 边界，均值 | 显式 Drop 主 coarse | 原顺序对照 |
| --- | ---: | ---: |
| SSG 总计 | 14.409s | 12.709s |
| SSG 阶段之外的销毁/尾部 | 3.041s | 1.398s |
| Vault 销毁 | 0.084s | 1.692s |
| **SSG 尾部 + Vault 销毁** | **3.125s** | **3.090s** |
| 完整组装 | 14.529s | 14.406s |

所以不能把 SSG 总计少了约 1.7s 宣称为整体加速：大部分成本移到了 Vault 销毁。主探针的 handle Drop 约 1.638s，也不能简单理解成“handle 自己的结构就需要 1.638s”。释放顺序、共享所有权及分配器整理/合并/trim 都可能影响成本落点；这次证明了成本迁移，没有分配栈证据去进一步区分这些子因。可靠的结论是临时对象及 Vault 收尾合计约 3.1s，而不是某个单独 Drop 标签。

## 5. 求值器：大量小操作，而非 Data 解析本身

以下为 **detail 三次均值**。Self 是该探针排除已计时子调用后的 wall time，包含未插桩工作和探针开销；不是 perf CPU Self。

| 位置 | 每次整站调用数 | detail Self |
| --- | ---: | ---: |
| `eval.present` | **8,492,164** | **3.608s** |
| `eval.execute` | **6,073,286** | **2.902s** |
| `eval.bind_content` | **4,426,913** | **1.870s** |
| `eval.implementation` | **6,073,286** | **0.894s** |
| HTML renderer | 51,739 | 0.801s |
| Notist 前端 compile | 51,739 | 0.348s |
| HTML registry | 51,739 | 0.203s |
| `data.native` | 1,955,784 | 0.154s |
| Data decode | 926,154 | 0.049s |

`eval.present` 的递归 inclusive 合计是 **83.655s**，明显大于进程 wall；这是嵌套范围重叠，不是额外花了 83 秒。不能把它或 `bind_content` 的递归 inclusive 加到总耗时中。

源码对应 `crates/notist-pipeline/src/code/eval.rs`：

- `execute` 每次准备参数、查参数定义、构造字段字典，部分调用还构造 Scope；再通过 `implementation` 查已检查实例。
- `implementation` 每次检查参数并生成实例 key，查询 program 的实例表；即使没有真正重新检查函数，也仍有数百万次通用路径。
- 内容在 `bind_content`、`present` 及调用边界之间被遍历、捕获、适配和物化。调用数量支持把这里作为进一步 CPU/分配栈采样的重点，但当前探针不能把所有 Self 再归因到具体 clone、比较或 malloc 指令。

不要把这些 detail Self 秒数与第 4 节 coarse 秒数相加。

### 缓存是否有效

| 观察 | 每次完整站点 |
| --- | ---: |
| Data read | 51,738 |
| 实际 JSON parse | **565** |
| parse cache hit | **51,173** |
| parse hit 比例 | **98.91%** |
| 模块 compile 请求 / 实际工作 | **2 / 1** |
| 模块 bind 请求 / 实际工作 | **51,741 / 3** |

52 个物品 JSON 块与 513 个目录 JSON 恰好解释 565 次实际 parse。detail 中实际 parse 总计 **0.098s**，Data decode 总计 **0.049s**，整个 `data.native` inclusive **0.366s**。这里需要优化的是调用、字段访问和内容处理的总体成本，而不是假设 51,225 个详情页都重解析了全量 Item 数据。

模块编译/绑定实际工作计数在 coarse/detail 的全部主样本一致。共享环境图与 comemo 已经把真正的模块工作压到很低；“再缓存 views.notc 编译”不是当前优先事项。

## 6. Python 扩展与文件系统

coarse 的 JSON command transport 均值：请求编码 **0.015ms**，子进程/管道交互 **205.205ms**，响应解码 **34.038ms**，transport 总计 **239.405ms**。准备阶段其他工作包含生成页面和路由的宿主处理。

独立 cProfile：Python 自身合计 **0.355s / 1,174,293 次调用**，主要 Self 为：

| Python 位置 | Self |
| --- | ---: |
| `parse_items` 校验记录 | 108.5ms |
| JSON `raw_decode` | 68.9ms |
| 字典 `get` | 43.4ms |
| `extend` 自身 | 29.0ms |
| `add_document` | 13.5ms |

cProfile 有明显开销，不能用 0.355s 替换正常 transport 的 0.205s 子进程时间。Python 是约百分之二量级的阶段，不是 18 秒的主要来源。

独立 `strace -f -c`（包括 Python 子进程和 CLI publish）记录选定文件/读写类 syscall：

| syscall | 次数 |
| --- | ---: |
| `statx` | **1,024,104** |
| `read` | **772,389** |
| `openat2` | 78,571 |
| `openat` | 26,540 |
| `close` | 105,031 |
| `getdents64` | 157 |
| 所选 syscall 合计 | **2,007,257** |

trace 的进程 wall **26.046s**，不能混进普通性能样本。trace 中各 syscall 的 `% time` 是所跟踪 syscall 时间的比例，不是整站耗时占比。少量失败调用包含可选路径和探测，构建本身成功。

源码中的重复祖先检查、regular-file metadata 检查、capability open 和有界读都值得继续细分。但这些统计包含完整输出比较，**不能把全部 200 万次 syscall 都归到 serve 组装**。例如 `src/resources.rs::FsReader::read_bounded` 的 `Vec::new()` + `take(...).read_to_end()` 可进一步研究预分配以减少小读；对 publish 的改善并不能解决 `Updated` 的主要耗时。任何改动都必须保留路径授权、大小上限、regular-file 检查和读前后 metadata 检查。

## 7. 内存

独立正常版 CLI 进程，每 20ms 读取 `/proc/<rust-pid>/status` 的 VmRSS；不使用会混入 Python 启动器历史高水位的 wait4 RSS 排名，也不把 Python 扩展 RSS 加进去。

| 该诊断进程的相对时刻 | Rust RSS |
| --- | ---: |
| 约 2s | 1,005 MiB |
| 约 4s | 2,420 MiB |
| 约 6s | 3,852 MiB |
| 约 8s | 5,276 MiB |
| 约 10s | 6,146 MiB |
| **11.497s，观测峰值** | **6,693 MiB = 6.54 GiB** |
| 退出前最后一次采样 | 6,428 MiB |

相比最终 451.8 MB 输出，这是很大的工作集，但 RSS 不是 live allocation，更不能直接证明内存泄漏或精确归因到 Environment、Item、Data。分配器可以在对象释放后保留页面；20ms 采样也可能漏掉更短峰值。

源码的整批分析结果、求值树、source/handle、环境与实例、完整输出字节有重叠生命周期，后续应以堆分配栈/保留对象图验证各自字节。当前只测了 CLI RSS，**没有测 serve 同时保留旧新两版站点时的 RSS 峰值**。

## 8. 优化优先级

1. **P0：求值器热路径与内容树表示。** 围绕 `present` / `execute` / `bind_content` 的千万级路径，进一步做无密集计时的 CPU/分配采样，减少重复捕获、临时节点及参数字典构造；研究稳定实例索引替代每次 key 构造和表查找。保留错误来源、预算、求值顺序与语义身份。
2. **P0：临时树销毁和约 6.54 GiB 工作集。** 优先验证 compact node、批量分配/释放、减少重复拥有的树。提前结束某些生命周期可降峰值，但单纯把 Drop 移到 `Updated` 计时之外不是加速，更不能泄漏对象换低数字。
3. **P1：减少 Data 的通用调用层成本。** `require_index` + `expect` 等产生大量小调用；可以研究检查后投影/accessor 的安全融合，而非重写 JSON parser 或把全量 Item 塞回 `site.data`。
4. **P1：跨 serve 重建复用已验证的不可变静态资产。** 约 333.7 MB 图标/索引每轮重新构造拥有字节；研究共享资产缓冲和可信失效机制。不能只信 mtime、跳过安全检查或把过期资产当缓存命中。
5. **P2：并行求值。** 潜在收益较高，但当前 ResourceSession / DataCache 的内部可变性、批次预算、资源读取和确定性错误顺序需要设计，不能直接给页面循环加 `par_iter`。

不优先：Tera（组装约 0.5%）、进一步缓存模块编译（实际仅 1 次）、JSON 解析器微优化（诊断约 0.1s）、Python 序列化微优化。按原顺序 coarse 对照约 50.1% 的求值份额，即使求值单独快一倍，整站组装理想上也只快约 **1.33 倍**；其它成本不会自动消失。

这些是按证据排序的方向，不是已经验证的优化收益。本次没有修改 Notist 生产实现。

## 9. 复现、完整性与限制

- 工具：`tools/profile_icexiv.py`；说明：`tools/ICEXIV_PROFILE.md`；新增测试：`tools/test_profile_icexiv.py`。
- `manifest.json`：确切版本、消费者状态/diff、输入、源码和二进制 SHA-256、机器和工具链。
- `samples.json`、`logs/*.json`：每个主进程的 wall/user/system、原始分层指标、caller edges；`summary.json` 为主批次汇总。
- `serve.json` / `logs/serve.stderr`：真实预览重建报告。
- `output-inventory.json`：全部 78,004 文件的大小和 SHA-256，每个成功 CLI 构建均核对。
- `python.pstats`、`strace.txt`、`memory.json`：分开的诊断证据，不参与正常耗时排名。
- `coarse.patch`、`detail.patch`、`sources/`、`bin/`、`reproducer/`：冻结插桩、源码、二进制和主批次脚本；独立析构顺序对照另存 `ordered.json`、`ordered.patch`、`ordered-reproducer.py`。

共完成 **21 次完整 CLI 构建及完整输出哈希核对、4 次真实 serve 组装**。最终校验确认输入、主/对照源码、二进制、重现脚本指纹及模块实际工作计数一致；结果写入 `integrity.json`。另运行消费者自己的 `scripts/check_site.py`，验证全部 51,225 详情、513 目录及所有本地 HTML 链接通过。

新增 profile 单元测试 2/2 通过。运行现有工具套件时，29 项中 27 项通过，两个已有历史插桩测试因对 `214de5f` 源码的锚点假设失效而报错；没有为本次分析修改这些历史实验。

这是分层 **wall-time profile**，不是 perf CPU flamegraph，也未采集硬件 cache miss、分配调用栈或精确 live heap。正式样本不含编译、文件哈希核对、cProfile、strace 或 RSS 采样。本会话不并发运行编译与计时，但没有隔离整机的其它工作；CPU 频率不锁定，也不声称具有微基准级的因果归因精度。
