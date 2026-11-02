# XIV DB 历程 profile

`profile_xiv_history.py` 重跑真实 XIV DB 的 18 个历史对照点；`xiv_history_probes.py` 只在结果目录的源码副本中加入阶段探针。不修改当前检出、消费者源码、依赖锁或生产额度。

```sh
python3 tools/profile_xiv_history.py prepare --out /tmp/opencode/xiv-history-NEW
python3 tools/profile_xiv_history.py build --out /tmp/opencode/xiv-history-NEW
python3 tools/profile_xiv_history.py measure --out /tmp/opencode/xiv-history-NEW --runs 5 --cpu 8
```

`prepare` 默认读取本机保存的历史输入、publication 路径补丁和 Handlebars 二进制；可通过 `--site-source`、`--xiv`、`--publication-patch`、`--handlebars-bin`、`--handlebars-templates` 指定。目录和 SHA-256 不符合预期会失败，不自动下载输入。只读 `Item.csv`，不传 `--limit`。

## 构建与口径

- 普通 CLI 每个引擎使用独立 Cargo target，避免 `git archive` 时间戳导致跨快照错误复用。所有普通二进制先保存，之后才构建探针。
- 探针使用独立源码副本；复用本版本的 target 前，显式清理该临时 target 内的工作区 crate 产物，只保留第三方依赖。每版的探针补丁和二进制指纹另存。
- release、debug=0、incremental=0、jobs=2；编译与正式测量不并发。
- 普通 CLI 每次新进程，完整生成、写盘并清理上一轮输出。先每版预热并验证，再五轮轮转／正反序测量；固定 CPU，Python 子进程继承亲和性。
- 记录 wall、user、system 和 Linux `wait4.ru_maxrss`（KiB）。RSS 不是进程树同时驻留内存总和。温文件缓存、不做 fsync、不固定 CPU 频率，不代表冷盘或持久化写入速度。
- Tera 及以后所有版本检查完整输出路径和逐文件 SHA-256；Handlebars/Tera 对 HTML 的解析事件（仅归一化空白缩进、属性顺序、字符实体）及非 HTML 字节进行比较。每版额外执行消费者全站校验器，检查所有物品、目录分页和本地 HTML 链接。

## 兼容与限制

- 早期 Tera 提交恢复当时已经存在但未提交的 publication 路径检查优化，不能与干净旧 commit 混淆。
- 历史 HEAD 对照点固定为 `2468767`，不随后续提交漂移；`working` 则拍下运行 prepare 时的当前检出及未提交改动。
- Handlebars 使用经过指纹校验的历史存档，完整未提交源码没有恢复，作为补充参照；并非可从 `574db84` 干净重建的严格源码对照。
- 资源版本只在临时 CLI 中加入 `XIV_PROFILE_REQUESTS` 覆盖，生产默认仍为 16,384。全量成功对照统一为 1,000,000 请求，仍经过授权、版本检查、读取及计费；另测默认额度失败，不把失败当成提速。
- `memo-off`、`memo-compile` 是同一 comemo 提交的显式消融，不是独立历史提交。带 profiling feature 时另设相应的 `CacheMode`，避免测试开关被忽略。
- 阶段探针三轮，时间是**包含式**：`ssg.*` 包含其 `pipeline.*`、`body.*`、`template.*`；`vault.*`、模块 compile/link/bind/install 也可能互相包含，不能全部相加。阶段排名不使用探针 CLI 代替普通 CLI。
- 大对象释放通过父阶段减去已测子阶段的逐样本残差估计：`ssg.local_release` 是 SSG 局部对象释放及未分配给子阶段的尾部工作；`cli.prepare_vault_release` 同时含宿主准备、观察结果收集及 Vault 释放；`publish.other_cleanup` 同时含发布安全检查、目录交换及旧输出清理，不声称是独立的 allocator profile。
- 同进程 comemo 测试使用一个带 profiling feature 的二进制，三种模式各三次进程，每次三轮新建 Vault，保留进程级计算缓存。不把新启动 CLI 称为“热 comemo 缓存”。XIV DB 没有 notc 导入时，只能报告实际 compile/bind 执行次数，不能推断模块密集型负载收益。
- 同进程 example 直接读取完整 Notist.toml，CLI 使用内存合成的 target 配置；只在各自宿主内部比较，不能把两者的绝对组装耗时相减。最后一次完整输出副本在组装计时之后写盘并逐文件校验。
- 本轮生产模板站点为 51,225 详情 + 513 目录 + 首页 = 51,739 HTML。另一次 notc 数据驱动正文验收属于不同页面模型，不能混入此时间序列。

## 结果文件

结果目录包含 `manifest.json`、`start.diff`、`sources/`、`profile-sources/`、`bin/`、固定 `site/`、历史 `scripts/`、`logs/`、逐文件 `inventories/`、`runs.json`、`profiles.json`、`summary.json` 和 `summary.csv`。构建或测量失败可重跑对应 action，已完成样本保留；需要改变测量条件时使用新的结果目录。

工具自身的轻量回归：

```sh
python3 -m unittest discover -s tools -p 'test_xiv_history_probes.py' -v
```
