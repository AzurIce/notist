# SourceId 校验重复遍历修复

## 改动范围

生产改动仅位于 `crates/notist-core/src/source.rs` 的
`Sources::validate_references`：检查 Node 自身的 origin/expansion 后，
表达式节点只沿表达式 IR 遍历，具体内容节点才沿 `children()` 遍历。

`Node.children()` 会投影出 `Call.children → Content` 内的节点，不能再与
`as_expr()` 并行作为两个独立入口。修复没有取消任何来源检查，也没有更改
SourceId 位宽、身份回收、修订冲突、缓存、wire、执行预算或 consumer。

本轮没有做 scratch/COW 优化，也没有推进 DefId；不自动提交。

## 回归测试

新增三项测试：

1. 12 层字面量调用嵌套，并加一个具体内容根节点。先验证内容投影与 IR 指向同一
   子节点，再精确断言只访问 14 个 Node、24 个 Expr。访问计数仅在 `cfg(test)`
   启用，正常 release 不含该探针；不使用脆弱的耗时阈值。
2. 分别遗漏叶节点 origin、expansion、Content 表达式来源、Call 表达式来源、
   Invocation 来源和外层 Node 来源，必须全部报 Missing。
3. `children()` 为空的调用仍检查参数内的 If 条件及两个分支，以及 For 的 subject/body。

曾临时恢复旧的双入口逻辑，第一项测试确实失败：访问量为 8,192 Node / 8,190 Expr，
不是 14 / 24。随后恢复修复并运行完整测试。

## 测试验收

- `cargo test --workspace --all-features --locked --offline`：794 项通过。
- `cargo test --workspace --no-default-features --locked --offline`：778 项通过。
- 独立 `notist-core` / `notist-eval` / `notist-pipeline` 无默认特性：159 项通过。
- `python3 -m unittest discover -s tools -p 'test_*.py'`：61 项通过。
- `git diff --check` 通过。

## 正式 clean A/B

证据目录：`/tmp/opencode/notist-source-walk-fix-20261108`。

修复前使用上轮完整 SourceId 生产实现的冻结、无插桩二进制，非永久池原型，
也非原始路径实现。修复后从本轮冻结源码重新编译无插桩 release，参数与上轮一致。
与修复前源码相比，既有 Rust/Cargo 文件仅 `source.rs` 改动。

复用上轮冻结的输入站点，不复制或修改 consumer。两版本分别预热一次，再做
8 对交错完整构建；每次都在计时外逐项核对 78,004 项产物、451,812,631 字节。
正式耗时不采用外部去重诊断副本，也不包括编译、访问计数或分配探针。

结果：

| 8 对 clean，中位数 | 修复前完整 SourceId | 修复后 |
| --- | ---: | ---: |
| wall | 9.8744 s | 9.4636 s |
| user CPU | 9.0395 s | 8.5768 s |
| system CPU | 1.1380 s | 1.1303 s |
| 峰值 RSS | 3,321,370 KiB | 3,322,228 KiB |

配对 wall 比值中位数为 **−4.16%**，8 对均更快；user CPU 中位数约下降 5.12%。
峰值 RSS 相差约 +0.84 MiB，没有明显变化。这次对照只量化修复相对于有重复遍历的
完整 SourceId 实现的收益，不与其它轮次的原始路径基线中位数拼接。

包含预热的 18 次完整构建产物全部一致。复测结束复核输入、冻结源码、二进制哈希
均未变化，生产 Rust/Cargo 文件仍与本轮冻结源码一致。

具体结果见证据目录的 `clean-summary.json`；所有运行日志、源码清单、二进制哈希、
本轮差异和复测脚本均保留。本项修复已完成验收，仍未提交或推送。
