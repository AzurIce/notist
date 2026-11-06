# 来源文本共享：Origin / SourceLocation（2026-11-05）

接续 [present checkpoint](2026-11-04-present-checkpoint.md)。恢复备份消除后，继续单独验证剩余真实构造及来源传播，不改变 Scope、展开链 Vec、整树所有权或全站保留生命周期。

## 实现

`Origin.source`、`SourceLocation.source` 从 String 改为既有的 `shared::Text`（Arc<str>）。来源值与跨度仍完整保留，clone 只共享不可变来源文本：

- bind/capture 复制来源名时不再分配字符串；
- expand 仍按原次序追加完整 SourceLocation，但来源名共享，Vec 及各节点的跨度仍独立；
- materialize 入口将来源名转换一次，整次递归构造复用这份 Text；
- 从 String 创建来源名时使用借用字符串直接构造 Text，避免先 clone 一个 String 再转 Arc；
- Diagnostic、TypedExpr、Invocation 的 source 类型不改；需要拥有诊断字符串的边界显式 to_string，查询／路径接口借用 as_str；
- HTML、SSG 的来源键保留文本值比较，不用 Arc 地址作为语义身份；没有全局 interner、缓存执行结果或跨版本来源表。

Text 只负责共享，不把两个独立创建的相同字符串强制合并。其生命周期随所有者结束，没有额外的全局强持有。旧的来源文本、展开链顺序、相对链接基准和数据错误的精确版本源码必须保持不变。

没有把整个 Origin 包成 Shared：每次挂载都会追加展开来源，粗粒度 COW 仍可能逐节点分离整个 Origin。这里共享不变的文本、保留可变 Vec 的独立所有权，避免把需要修改的部分也无条件套进 Arc。

### Rust API 迁移

wire 中 source 仍是普通 JSON 字符串，Debug 也保持原字符串显示；**Rust 字段类型不源码兼容**：

```rust
// 已有 String 可用 .into()；已有 Text 优先 clone。
let source: notist::Text = path.as_str().into();
let location = notist::SourceLocation {
    source: source.clone(),
    start: 0,
    end: 4,
};

// 普通读取无需拥有字符串。
let path = std::path::Path::new(location.source.as_str());

// 需要 String 的外部接口显式转换。
let source_name: String = location.source.to_string();
```

需要修改来源名时替换 Text 值；不能继续使用 String 的原地 push_str 等可变方法。Root 的手写 JSON／preview source-map 投影使用 as_str，因此不需要为无默认特性的构建强制开启 core serde。

## 正确性验证范围

新增回归检查 Origin 克隆共享来源文本但不共享 Vec 的修改、独立存储按文本和完整跨度比较、Unicode／引号路径的 wire 与 Debug 值，以及一次 materialize 中节点的来源共享。既有 1,792 组 checkpoint 递归差分、读取／预算／Pending／错误来源和多前端回归继续执行。

本批只有 12 个生产文件变化：两个来源字段及其构造／读取／投影边界。消费者内容、Cargo.lock、依赖特性、请求额度、逻辑执行预算和公开语义树未改变；不把布局缩小变成更宽松的逻辑额度。

## 测量边界

证据目录：`/tmp/opencode/notist-source-text-20261105/`。before 是上一批 after 的逐文件相同副本，沿用其 736／745 项 workspace 测试日志，不声称再次执行基线测试；普通两版二进制均在本批重建。after 只覆盖明确列出的 12 个文件。

工具 `tools/profile_source_text.py` 复用前述冻结与 A/B 协议，分别测：

1. 固定容量 Rust System wrapper：item_shell 的字段 clone，以及 expand 追加来源链的分配流量。它不是正常二进制，不用于加速或峰值内存结论。
2. 普通无插桩二进制：warmup 后六轮交替顺序的整进程配对，包含构建、现有输出比较及退出。
3. 两次独立 Rust PID VmRSS 采样；不与主计时合并。
4. 额外的 glibc malloc/calloc/realloc/free API 观察器：转发给相同 allocator，按可执行文件路径排除 Python，只比较整进程请求流量，不用其耗时计算加速。

本批字段基线中，bind/capture 的 Origin clone 有 **4,277,854 次来源名分配、278,559,092 请求字节**；expand 有 **4,169,741 次来源追加、8,339,482 次分配事件、826,325,513 请求字节**。请求字节含 Vec 容量，不是实际复制字节或 live heap；来源路径含本批目录，不跨批混算。

所有完整构建核对全部 78,004 个输出文件，另比较四组完整 analysis/evaluation JSON、检查站点链接。源码、输入、脚本和二进制都有指纹。共享是否改善耗时与 RSS 由本批结果决定，不继承前几批数字。

## 验收结果

### 字段归因与净分配

bind/capture 的来源 clone 次数保持 **4,277,854 次**，但该范围的分配从 **4,277,854 → 0 次**。expand 仍追加来源 **4,169,741 次**，分配从 **8,339,482 → 4,169,741 次**，请求字节从 **826,325,513 → 400,295,136**；剩下的是独立 Vec 的分配，而非逐节点复制来源字符串。其它场景／字段计数不变，没有少构造节点或省略来源传播。

额外的普通二进制 glibc API 观察器确认了整进程净收益，包含创建 Text 本身的成本：

| 请求流量 | before | after | 减少 |
| --- | ---: | ---: | ---: |
| 分配／重分配事件 | 219,010,110 | 210,347,616 | **8,662,494，3.96%** |
| 请求字节，十进制 GB | 20.305 | 19.446 | **0.859，4.23%** |

这不是堆驻留量；API 观察器不覆盖 libc 内部及所有 aligned 分配入口，realloc 计完整新大小。Python 子进程被显式排除；该组耗时不用于性能结论。

### 无插桩配对耗时

| 六轮中位数 | before | after |
| --- | ---: | ---: |
| wall | **14.823s** | **13.781s** |
| user | 12.916s | 12.008s |
| system | 2.186s | 2.023s |

两个 wall 中位数少 **1.042s**。逐轮配对降幅为 `11.37 / 7.66 / 5.27 / 5.27 / 10.61 / 6.84%`，配对中位数 **7.25%**，六轮均更快。六样本配对中位数的描述性 bootstrap 95% 区间为 **5.27–10.99%**，不证明其它机器／负载也有相同区间。

正式进程合计发生 23 次 major fault，非自愿上下文切换约 834–1,696 次；CPU 频率与其它任务未完全隔离。保留全部样本，不把本批与前批目录／环境下的绝对秒数直接相减，也不相加各批收益。

### 这次确实观测到 RSS 下降

25ms 采样 Rust PID VmRSS 的两次独立峰值：

- before：**6.5063 / 6.5063 GiB**；
- after：**5.7431 / 5.7433 GiB**。

两次峰值中位数少 **0.763 GiB、11.73%**。与前几批只减少短命分配不同，来源文本在最终树和来源映射中也被共享，减少了整轮同时保留的重复来源缓冲。这里报告的是本负载的观测峰值 RSS，不是精确 peak live heap 或通用内存上限；采样仍可能漏掉短暂尖峰。

### 正确性

- after workspace 默认 **740 项**、全特性 **749 项**通过，均为 74 个 test target、零编译警告；根库／核心／管线无默认特性 **504 项**通过。
- 新增 4 项来源共享／身份／wire／构造回归；既有 1,792 组 checkpoint 差分及资源预算等回归继续通过。
- 字段观察器 Rust 单测 1 项、来源观察工具 Python 单测 2 项通过。
- **23 次完整构建**全部 78,004 文件 SHA-256 一致：普通验收 19 次、字段观察器 2 次、整进程 API 观察器 2 次。四组完整 analysis/evaluation JSON 前后一致；51,225 个详情页、513 个目录页和全部本地 HTML 链接检查通过。

最终复核源码、输入、二进制和脚本指纹；只清理本批外部 Cargo target 的可重建产物，保留完整证据与测量二进制。所有修改未提交。

## 仍未处理

Origin.expansion 仍是独立 Vec，来源传播遍历次数不变；实例身份、参数准备、真实树构造及全站生命周期没有由本次 Text 分享一并解决。后续依据剩余分配与驻留数据再决定是否评估持久来源链、身份共享或生命周期调整。
