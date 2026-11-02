# 2026-11-02 notc 编译与导入绑定缓存评估

## 范围

基线为模块分层重构提交 `268fb35`，依次比较仅启用单文件编译缓存、再启用导入绑定缓存。未缓存文档 Parse/Lower、实例检查、求值、扩展、模板或发布结果，没有修改 xiv-db 的源码、页面数量或依赖锁。

编译缓存键包含源码、模块身份、来源路径、逻辑根，以及前端实现类型／版本／完整配置。内置前端启用，自定义前端默认关闭。绑定缓存以 BindingSpec 为普通键，跟踪 ModuleExports.lookup/all；空规格直接共享空作用域，不经过 comemo。成功与错误都可复用。CLI Book/Site 装配及 LSP 输入批次间进行年龄 10 的全局淘汰；这不是字节预算或跨进程缓存。

## 方法

- 三个阶段各保留独立 release CLI 和 profile 二进制，先完成构建再运行测量，不将编译耗时计入。
- profile 使用可选 `profiling` feature：线程局部计数、阶段纳秒计时；普通构建没有这些计数／计时开销。
- 每个 profile 进程执行三轮，每轮创建新 Vault；首轮计算缓存为空，后两轮保留计算缓存。表中的“同进程后续”是后两轮的中位数，不等于完全冷机或冷文件系统。
- CLI 使用三个独立子进程，含发布写盘；不把独立启动称为热 comemo 缓存。
- 使用 Python wait4 测量子进程高水位 RSS，非整个进程树的内存总和。profile 峰值覆盖三轮；组装计时不含最后一次验证产物的写盘。
- Docs 与其相邻 modules 通过 `git archive 268fb35 docs modules` 固定输入，避免工作区并发文档编辑干扰比较。合成输入固定为 1,000 页、8 个包装模块和一个含 24 个函数的共享模块。
- 本机未固定 CPU 频率或亲和性；小于数毫秒的差异不作为显著收益结论。阶段时间为包含式，compile_work 属于 compile，bind 属于 link，不能直接累加所有列。

## 分阶段结果

同进程完整分析／求值或组装耗时：

| 负载 | 无缓存：首轮／后续 | 仅编译缓存：首轮／后续 | 两处缓存：首轮／后续 |
| --- | ---: | ---: | ---: |
| 1,000 页共享模块 | 1090.18 / 1024.97 ms | 985.16 / 966.39 ms | 969.58 / 954.36 ms |
| Docs，39 页 | 332.08 / 160.22 ms | 331.23 / 175.67 ms | 336.83 / 158.93 ms |
| IceXIV DB，51,739 页 | 2916.77 / 2867.84 ms | 2920.70 / 2882.95 ms | 2924.40 / 2867.48 ms |

profile 进程峰值：

| 负载 | 无缓存 | 仅编译缓存 | 两处缓存 |
| --- | ---: | ---: | ---: |
| 共享模块 | 57.1 MiB | 33.1 MiB | 22.9 MiB |
| Docs | 113.0 MiB | 112.9 MiB | 113.0 MiB |
| IceXIV DB | 823.8 MiB | 821.5 MiB | 825.1 MiB |

共享模块负载的后续轮次较基线约快 6.9%，峰值 RSS 约下降 60%；主要来自跨文档共享模块与作用域，不是缓存整个页面。仅编译缓存约快 5.7%，绑定缓存进一步减少重复装配工作。

实际执行次数（首轮 → 后续每轮）：

| 负载 | 无缓存编译／绑定 | 仅编译缓存 | 两处缓存 |
| --- | --- | --- | --- |
| 共享模块 | 2000 / 3000 → 不变 | 9 / 3000 → 0 / 3000 | 9 / 17 → 0 / 0 |
| Docs | 133 / 253 → 不变 | 7 / 253 → 0 / 253 | 7 / 18 → 0 / 0 |
| IceXIV DB | 0 / 51739 → 不变 | 不变 | 0 / 0 → 不变 |

共享模块负载首轮 compile 包含式耗时从 56.54 ms 降到 0.72 ms，bind 从 21.48 ms 降到 1.48 ms。Docs 的原始模块编译约 1.7 ms、绑定约 0.24 ms，本来就不是主要时间来源；整站时间没有足够明确的提升。IceXIV DB 当前由 Python 生成普通正文，没有 notc 导入；省下空绑定的几毫秒不能解释为全站提速。

独立 CLI Docs 全量构建中位数：无缓存 0.347 s，仅编译缓存 0.345 s，两处缓存 0.342 s。差异很小，不宣称得到明显的 CLI 加速，也不宣称跨进程复用。

另对 Ranim Book 的 26 页做三个独立进程的全量 CLI 构建：中位数依次为 0.266 / 0.251 / 0.247 s，RSS 约 79–80 MiB。三个阶段的 51 文件、6,416,930 bytes 产物完全一致，汇总 SHA-256 为 `215940b83e981d9115e969fa969628320417bf692568d712cb3619806319eb0a`。该小样本观察到约 7% 差异，但不将其中所有波动都归因于缓存；未涉及 Ranim 网站的生成／渲染命令。

## IceXIV DB 的默认预算限制

当前消费者目录为 `/home/azurice/Files/_ffxiv/icexiv-db`，即原 xiv-db。全量结果为 51,225 件物品、513 个目录页和首页，共 51,739 HTML。

基线及两种缓存版本的默认 CLI 均在发布前失败：会话 `requests = 16,384`，逐页重复读取配置等输入耗尽请求额度。缓存没有绕过宿主读取与计费，所以也不会“修复”这项限制。失败运行不是成功全量构建时间，不纳入加速比较。

为完成等量比较，profile 驱动在所有阶段统一使用 `InputLimits.requests = 1,000,000`，其余默认限制不变；组装包含 Python prepare 和所有页面，不包含发布事务。此限制仅写在基准驱动中，没有提高生产默认值。

两处缓存版完整产物通过消费者 `scripts/check_site.py`：全部物品、分页数量和本地 HTML 链接有效。三个阶段的文件名＋逐文件 SHA-256 汇总完全一致：

- Docs：73 文件、39 HTML、1,506,502 bytes；`d63658a6ab4f3e42d711865b84426d7a0cc2eb7595b0dfa31784233e4c53a059`。
- IceXIV DB：51,744 文件、51,739 HTML、114,923,532 bytes；`142799d06157ad506f95aa3c71c317fabb1e6b5ceb86d6d85909dd660299cf38`。

后续应单独梳理大站点的源码／配置请求计费与宿主可配置预算，不能以无条件增大硬限制或缓存命中不计费代替设计。

## 回归与复现

新增测试覆盖跨 Pipeline 复用、实际前端及配置、路径／根／源码失效、错误缓存修复、宿主 COW 修改不污染缓存、成员及通配导出变化、重导出目标变化、缺失成员恢复、签名／实现更新、淘汰后活产物有效，以及冷／热缓存保持相同宿主请求预算错误。

验证命令：

```sh
TERM=dumb NO_COLOR=1 cargo test --workspace --features profiling
cargo check -p notist --no-default-features
```

`--all-features` 会启用一项既有 serde 测试失败：`option_values_round_trip_and_key_bit_exact` 用 PartialEq 比较 Some(NaN)，而 NaN 不等于自身；相同断言存在于基线。跳过这一项后，其余全特性测试通过。本轮没有修改 Value 相等语义或掩盖该失败。

测量设施：`crates/notist-cli/examples/module_cache_profile.rs`、`tools/profile_module_cache.py`。例如：

```sh
cargo build -p notist-cli --release
cargo build -p notist-cli --release --features profiling --example module_cache_profile
python3 tools/profile_module_cache.py --label current \
  --cli target/release/notist --profile target/release/examples/module_cache_profile \
  --xiv /home/azurice/Files/_ffxiv/icexiv-db
```

profile 驱动还支持 `--cache-mode baseline|compile|both`，在同一个带 profiling 的二进制中分别关闭两处缓存、仅启用编译缓存、启用两处缓存，便于后续重现阶段计数。该开关只影响 profile 子进程，CLI 对照仍须传入对应版本的二进制；普通构建不包含运行时开关。每次使用新的 label，工具拒绝混入上次遗留产物。

原始三阶段 JSON、stdout/stderr、release 二进制和验证产物位于 `/tmp/opencode/notist-module-cache-profile/`；该路径是临时测量记录，不是持久化缓存。
