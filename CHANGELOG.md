# Changelog

版本号以 `Cargo.toml` 里的 `version` 为准；发布由推同版本的 `v*` tag 触发（见
`.github/workflows/release.yml`），因此「有 tag ＝ 已发布」。

## [0.3.19] - 2026-10-25

### 变更

- Python 服务包安装和升级统一使用 `uv pip install`，不再调用 `uv tool install`。
  已有 CLI 继续使用原解释器，旧的 `uv tool` 环境保留查询和卸载支持；首次安装需要虚拟环境。

## [0.3.18] - 2026-10-21

### 变更

- **`fungame-web` 的后端换成新包名。** 后端由 `fungame-backend` 更名为 `fungame-api`，
  包名与命令同名（`fungame-api server …`）；启停参数、端口（8809 / 8807 / 8808）不变，
  `FUNGAME_WEB_BACKEND_PACKAGE` 仍可覆盖包名。配置目录改为 `~/.farfarfun/fungame/api/`，
  fundeploy 仍通过 `fungame-api server status` 输出里的 `config <路径>)` 取得它。
  旧包 `fungame-backend` 不再维护，旧装机需要手动装 `fungame-api` 并停掉旧服务。

## [0.3.17] - 2026-10-20

### 变更

- `paperclip` 新装正式版和 canary 默认通过 npm 镜像安装到用户目录，省去首次 `npx`
  引导与 managed install 的重复安装；镜像失败自动回退官方源。后续升级沿用安装
  通道和成功使用的源。已有 managed install 继续使用原路径；npm 全局安装不支持
  managed payload 的 `rollback`。

## [0.3.16] - 2026-10-20

### 变更

- 交互菜单进入 `service` 后直接显示状态总览和所有服务；选择器一次显示最多 15 项。

## [0.3.15] - 2026-10-20

### 变更

- 普通 Python 包安装统一使用 `uv pip install`，更新时加 `-U`；保留 `uv tool`
  管理独立 CLI，以及指定版本、回退时的依赖处理。安装相关服务现在需要 `uv`。

## [0.3.14] - 2026-10-19

### 变更

- **`fungame-web` 的两个前端换成新包。** fungame 的管理端和用户端不再各有一个
  独立的部署载体包，应用包本身就是部署制品（只含构建产物 `dist/` 与零依赖的静态
  服务 CLI），自 0.1.47 起发布：
  - 管理端 `@fungame/admin-cli` → `@farfarfun/fungame-admin`，命令仍是 `fungame-admin`；
  - 用户端 `@fungame/client-cli` → `@farfarfun/fungame-web`，命令由 `fungame-client`
    改为 `fungame-web`。

  启停参数不变，端口仍是 8809 / 8807 / 8808。

## [0.3.13] - 2026-10-17

### 变更

- **`skills sync` 改成并发的。** 这是本次唯一收益可观的改动。每个技能仓库要一次
  git 网络往返，实测到 Gitee 单次约 800 ms，而二十四个仓库彼此毫无关系 —— 串行跑
  就是把等待时间一个个加起来。实测（用只读的 `git ls-remote` 量纯网络部分）：

  | | 串行 | 并发 8 |
  | --- | --- | --- |
  | 8 个仓库的 git 网络往返 | 4.4 s | 0.5 s |

  按此外推二十四个仓库约从 13 s 降到 2 s 量级。这条命令的全部价值就是「一次把全部
  拉齐」，不该让人按着 Ctrl+C 犹豫要不要等下去。

  并发之后 git 的输出必须捕获：八条 git 同时往终端写，而 git 的进度条还会用回车
  重画当前行，交错之后连已经打出去的行都会被盖掉。所以同步报告改成等各仓库做完
  之后**按远端列表的固定顺序**出，不随完成顺序走。点名装一个（`skills install X`）
  那条路不变，仍然让 git 的进度直通终端 —— 那时候终端是它一个人的。

  顺带把「目录被别人占着」从 `Err` 改成一种正常结局（`Did::Skipped`）。对 sync 来说
  那是预料之内的局面（人自己在那儿放了东西），不该让整条命令以非零退出；点名装一个
  时它才是失败。两种判断现在靠类型区分，不靠去匹配错误里的字。

- **新增 `common::par`，三处并发收成一处。** 换源测速和 GitHub 连通性诊断本来各写了
  一遍线程池，技能同步和工具链状态是第三、第四处。`par::map` 保证两件事：结果按
  **输入顺序**返回（按完成顺序收的话，「最快的那个源」配出去的会是另一个源的地址，
  而这种错不报错、只会悄悄配错），以及**抢任务而不按线程数均分**（一件慢活不该拖住
  和它分到同一份的其他活）。`source/probe.rs` 的 `race` 因此从 25 行降到 3 行。

- **`fundeploy dev` 的五条工具链并发探。** 175 ms → 133 ms。耗时大头是
  `pnpm --version` —— 装了 corepack 的机器上那是 corepack → pnpm-native 三层嵌套。
  为此把「探」和「打印」拆成了两步，顺序输出不变。

- **`skills list` 的仓库状态并发查。** 热缓存下 1.42 s → 1.29 s（9%），收益有限：
  这条命令的大头是那一次 Gitee API 调用（约 1 秒，而且只发一个请求 —— `PER_PAGE`
  是 100，二十四个仓库一页就够，没有可省的往返）。冷缓存下好一些：二十五次
  `git status` 第一次跑要 2.3 s，热了之后只要 230 ms。

## [0.3.12] - 2026-10-17

### 修复

- **`rust-version` 写的不是真实下限。** manifest 里写着 `1.74`，而锁住的依赖里
  `icu_*`（`ureq` → `url` → `idna` 那条线上的）要求 1.88 —— 也就是说这一行从写下
  起就是假的。后果不是编不过，而是**编不过时报的是别人的名字**：拿 1.80 的人读了
  manifest 以为自己够格，`cargo install` 装到一半收到一句「package `icu_normalizer`
  requires rustc 1.88」，跟 fundeploy 看着毫无关系。那行注释原本的用意恰恰是「让
  版本不够时直接报清楚」。

  现在写 `1.88`，并在 CI 里加了一个 `msrv` job：它从 `Cargo.toml` 读这个数字、装
  那一版 rustc、`cargo check --locked --all-targets` 真编一遍。从 Cargo.toml 读而
  不是在 workflow 里再写一遍版本号 —— 两处各写一份必然漂移，而漂移的方向总是
  「CI 还在测老版本，manifest 已经抬上去了」。

  顺带去掉了两个白抬下限的依赖，见下。

### 变更

- **端口查询改成批量的。** `holders()` 和 `is_taken()` 是单端口 API，却一直在端口
  列表上被循环调用。Linux 上认占用者要走两步：读四张 `/proc/net/*` 表拿 socket
  inode，再遍历 `/proc/<pid>/fd/*` 反查谁持有它们 —— 第二步是对**全机每个进程的
  每个 fd** 做一次 readlink。照端口数重复做的话，`port-kill 8080 8081 8082` 就是
  三遍全盘扫描，`service status` 的总览（十几个服务、二十几个端口）是二十几遍四表
  解析。

  新增 `holders_many` / `taken_many`，四张表读一遍、`/proc` 走一趟，要问几个端口
  都一样；单端口那两个现在是它们的特例，两边不会再漂。空闲开发机上实测一遍全盘
  扫描只要 4 ms（547 个 fd），所以这不是给开发机省时间 —— 这条命令真正要用的地方
  是跑着十几个服务的机器，那里进程数和 fd 数高一两个数量级。

  `force_free_port` 顺带少一趟：它此前先 `listener_pid()` 再 `who()`，两次 `/proc`
  全盘扫描紧挨着，第二次问的还是第一次已经看过的东西。

- **不再依赖 `which` 和 `zopfli`。**

  `which` 整个项目只用在两行（`cmd::has` / `cmd::find`），却拖进 `rustix 0.38` +
  `linux-raw-sys 0.4` —— 依赖树里另有一份 `rustix 1.x`（clap、tar 在用），同一套
  syscall 封装要编两遍。换成十几行 PATH 遍历：本项目在 `platform.rs` 里就明确只认
  linux 和 macos，不存在 Windows 的 PATHEXT 问题。它原先保证的语义（带 `/` 的当
  路径、要有执行位、目录不算、跳过 PATH 里的空项）现在由本仓库自己的六条测试钉住。

  `zopfli` 来自 `zip` 的 `deflate` feature，而那个 feature 等于
  `deflate-flate2 + deflate-zopfli` —— zopfli 是个**压缩**器（拿时间换压缩率），
  本仓库只解包，一行都用不到它。改成精确的 `["deflate-flate2", "flate2"]`，用纯
  Rust 后端（不是 `deflate-zlib`：musl 全静态构建不该去链系统 zlib）。

  合计：依赖从 199 个降到 192 个，`rustix` 和 `linux-raw-sys` 两组重复版本消失。
  **二进制反而大了 15 KiB**（3267 → 3282 KiB），所以这条换的是编译时间和依赖面，
  不是分发体积。

- **拆开 `paperclip.rs`。** 3163 行一个文件，其中 `impl Paperclip` 一个块就 1284
  行、近五十个方法。按「跟 Paperclip 的生命周期是不是同一件事」切成四块：
  `prefetch`（npm 预取与安装期进度测量 —— 上游把 npm 的输出吞了，只能从外面观察）、
  `progress`（进度条样式，没有任何 Paperclip 语义）、`plugin`（会随上游生态增补的
  静态表）、`log`（托管那条路的日志）。方法跟着自己那摊事走，测试跟着它测的东西走。
  主文件 3163 → 1640 行，`impl Paperclip` 1284 → 796 行。纯搬运，没有行为改动。

- **`cli.rs` 里抄了十四遍的那条链收成一处。** `ui::select(...)?
  .ok_or_else(|| anyhow!("已取消。"))?` 出现十四次，「取选中行的第一个词」出现九次
  且默认值各不相同（`"status"` / `"list"` / `"诊断"` / `""`，其实都取不到）。现在是
  `choose` / `choose_head` / `choose_index` 三个小函数，顺带去掉了两处
  `position(...).unwrap()`。

- **`uninstall_env_var` 的两份实现并成一份。** service 和 tool 两域各写了一遍，连
  测试都抄了两遍。那个变量名是对外承诺 —— 无人值守脚本里写着它们，两份实现漂开
  一点，脚本就会静默卡在确认上。

- **测速不再为每个源重建一次 HTTP agent。** `source test` 此前每个 URL 新建一个
  `ureq::Agent`，等于把 rustls 的 ClientConfig（含根证书集）重新装配十几遍，而量到
  的数字里就掺进了这部分本机开销 —— 这条命令的全部意义恰恰是比较各个源之间的差别。

### 工程

- **clippy 规则写进 `Cargo.toml` 的 `[lints.clippy]`。** 此前规则只活在 CI 的
  `-D warnings` 参数里，本机 `cargo clippy` 拿到的是另一套。现在同一张表在本机是
  warn、在 CI 是 deny。

  收录标准是「存量为零且拦得住真问题」，一共十条，其中 `dbg_macro` / `todo` /
  `unimplemented` 拦的是「没写完的东西发出去了」—— fundeploy 是装到别人机器上执行
  的命令行，`todo!()` 在这里不是占位符，是用户终端里的一次 panic。没有整组打开
  pedantic：那组 225 条里绝大多数是 `must_use` / `doc_markdown` 这类噪音，真要清掉
  得改三十个文件，换来的多半是更难读的写法。

- **CI 的 cache key 带上 rustc 版本了**，`restore-keys` 也带。stable 是滚动的，只按
  `Cargo.lock` 做 key 的话，`restore-keys` 会把上一个 rustc 编出来的 `target/` 捞
  回来 —— 升级那周的表现是莫名其妙的重编，或者更难查的链接错误。

## [0.3.11] - 2026-10-16

### 变更

- **`fundeploy about` 带上分发渠道了。** 此前只有组织主页、PyPI 和联系方式，没告诉
  人怎么装 —— 而那正是 [farfarfun/packages](https://github.com/farfarfun/packages)
  的用处：那边一个仓库同时是 apt 源和 Homebrew tap，管全组织的包，fundeploy 发版时
  只是通知它重建索引。现在 `about` 分三节列清组织、分发、本项目，底下再给出可以直接
  复制的装机命令（装机脚本和 brew 两条），以及 `brew trust` 和 Gitee 备用地址这两个
  一定会碰上的坑。

  装机命令摆在卡片**外面**：它们比框里任何字段都长，塞进去只会把框撑到满屏，或者在
  窄终端下被中间省略成认不出的样子 —— 而这是要被复制去敲的东西，掐掉就没用了。

## [0.3.10] - 2026-10-16

### 修复

- **Homebrew 渠道升级成功却报失败。** `brew upgrade` 升完 fundeploy 之后会接着处理
  别的 formula，那一步失败会让整条命令退出非零 —— 而 fundeploy 把这个退出码当成了
  自己的失败。实测到的一次：fundeploy 已经 `🍺` 到 0.3.9、旧的 0.3.8 也清理掉了，
  brew 紧接着去下另一个 formula 的 `.rb`，撞上 `raw.githubusercontent.com` 连不通
  （国内常事），于是屏幕上「升级成功」和「错误: 命令失败（退出码 1）」同时出现。

  根因是退出码**两头都不可信**：`brew upgrade` 在「没有更新可升」时退出 0，顺带升
  别的 formula 失败时又退出非零。真正的答案只有一个来源 —— `brew list --versions`。
  这一点在 0.3.4 加核实逻辑时只考虑了前一半（「退出 0 不等于升上去了」），漏了后一
  半（「退出非零也不等于没升上去」），而那段核实恰好排在 `bail` 之后，根本没机会跑。

  现在不论 brew 成败都先问一次它装着哪一版：已经是新版（或本来就是最新）就按成功
  报，并把 brew 那条抱怨单独说明是别的 formula 的事；确实没升上去才报失败，并把
  Homebrew 当前的版本一起说出来。

  apt 和 cargo 那两条路没有跟着改：apt 的索引刷新失败早就是「只警告不中断」，而
  `apt-get install` / `cargo install` 本身失败就是真装不上 —— 没有实测到同类情形，
  不跟着扩散改动。

## [0.3.9] - 2026-10-16

### 修复

- **交互菜单现在能走到每一条命令。** 在此之前菜单只是顶层那十来个域的入口，再往下
  一层的命令有不少**在交互里根本到不了** —— 而菜单正是「不知道有哪些命令」时唯一
  能翻的地方，到不了就等于不存在。补齐的有：

  | 域 | 以前到不了的 |
  | --- | --- |
  | `service` / `tool` | 不点名的 `status`（所有服务/工具的状态总览） |
  | `service` | `rollback`（回退到指定版本） |
  | `source` | `add`、`rm`（手写菜单只列了 5 项，实际有 7 个） |
  | `ai` | `status` / `install` / `upgrade` / `uninstall` 全部（菜单只列表） |
  | `dev` | 同上四个（同样只列表） |
  | `skills` | `sync` / `install` / `update` / `rm` |
  | `doctor` | `--fix`、`--undo` |
  | `upgrade` | 真升级（只能看有没有新版） |
  | `port-kill` | `--list`（只看谁占着、不动手） |

  `service` 和 `tool` 现在先问一句「看全部状态，还是挑一个」；`ai`、`dev` 先选哪个
  工具（或「全部」）再选动作；需要带值的动作（`rollback` 的版本号、`source add` 的
  地址、`skills install` 的仓库名）选中之后再问，不填就报错而不是拿着空值往下走。

  `upgrade` 在菜单里默认停在「只看有没有新版」：菜单是随手点开的，真把自己替换掉
  该是个明确的决定。

- **每个域的动作表成了唯一出处。** 这些菜单从前是手写的第二份清单，和 clap 的定义
  各写一遍，于是漏了也没人知道 —— `rollback` 就是这么漏的：拼错子命令时的报错文案
  认得它，菜单里却找不到。现在菜单由动作表生成，并有测试拿它和 clap 的子命令**双向**
  对照，哪边多一个少一个都会红。

## [0.3.8] - 2026-10-16

### 修复

- **`fundeploy service status` 和 `fundeploy tool status` 回来了。** 旧 shell 版帮助
  文案的示例第一条就是 `fundeploy service status`，改写成 Rust 时丢了：clap 把
  `status` 正确解析成动作、服务名留空，而分发只看服务名，于是把解析出来的动作整个
  丢掉，打了张服务清单还退出 0 —— 看着像成功了，其实问的事没人答。

  现在它给所有服务一份状态总览（在跑 / 没在跑 / 部分 u/n）。只探端口、不碰 PID
  文件：这条命令的用处是「扫一眼这台机器上什么在跑」，得快；单个服务的 PID、版本、
  日志路径仍然归 `fundeploy service <服务> status`。

- **不点名服务的危险动作不再静默变成「打印清单」。** `fundeploy service stop` 以前
  和 `service status` 走同一条死路，现在明确报错并给出用法。**不**对所有服务执行是
  故意的：它看着像「停一下」，真跑起来是把这台机器上十几个服务全停了。

- **无参数菜单补上了 `upgrade`、`list`、`about`。** 这三个此前只在 `--help` 里有，
  菜单里没有。菜单那个 `_ =>` 兜底分支也删了 —— 它会把「加了菜单项却忘了加处理
  分支」变成「选了 upgrade 却跳出工具列表」，不报错、也查不出来。菜单现在提成常量，
  由一条双向测试钉住它和 clap 的命令定义一致，漏哪边都会红。

### 变更

- **输出重做成「一张卡片说完一件事」。**

  ```
  ╭─ Paperclip ─────────────────────── ○ 未运行 ─╮
  │  fundeploy 托管  ● 没在运行 · PID 文件已失效 │
  │  版本            2026.1014.0-canary.11       │
  │  端口 8804       ○ 没监听                    │
  ╰──────────────────────────────────────────────╯
  ```

  框宽跟着终端走（问不出来按 80 列，上限 88），放不下的值**中间**省略而不是截尾
  —— 这里装的多半是路径，掐掉尾巴就只剩一串目录名，留头留尾才看得出是哪个文件。
  36 列的窄终端下框会缩到 34 并照样对齐，不会被硬折成满屏散落的框线。

  **输出不是终端时（重定向、CI）退回无框的纯文本**：框线和色码进了日志只是噪音，
  而那时也问不出终端宽度。`NO_COLOR=1` 同样一个色码都不出。

  一张卡片里混着两类字段时会分节 —— 前后端服务的「包」和「运行」两组键名几乎一样
  （都是「后端 funfluid-api」这种），不分节看起来就是同一组东西重复了两遍。

  `status` 里不再用警告通道：「进程在但端口没监听」是**状态本身**，不是操作中途出的
  意外。分到 stderr 会让一次 status 的输出一半进文件、一半留在屏幕上，顺序还会因为
  两个流各自缓冲而错乱。真正的警告留给 install / start / stop 那些「正在做事，出了
  点意外」的场合。

- **等端口监听不再是静默干等。** 这一等最长到半分钟（装了一堆依赖的 Python 后端
  import 完就要这么久），期间屏幕上一个字都不动，跟卡死没有区别。现在转一个 spinner
  并报已过时间，进度画在 stderr —— `fundeploy ... > out.txt` 里不会混进动画残渣。

- **配色补齐。** 此前只有四处上色，而用得最多的那几个输出函数全是纯白：最该一眼看出
  的「没在运行 / 未安装」和路径、版本号一个颜色。现在状态有四种成色（在跑绿、该在跑
  却没在跑红、半开半合黄、本来就没有灰），路径和提示转暗，分节标题换成色块竖线，
  交互菜单也配上了同一套色板。

### 新增

- **`fundeploy about`**：这是什么、谁做的，以及 farfarfun 的组织主页、PyPI 和联系
  方式。无参数菜单顶上也带一段，但**只有这两处** —— 每条命令都顶一段 banner 很快就
  会变成噪音，而 `service x status` 的输出是会被重定向进文件、被 grep 的。

      fundeploy 0.3.8  开发环境、工具与服务管理
      farfarfun  活到老 · 学到老 · 玩到老 · 老牛老牛了

## [0.3.7] - 2026-10-16

**这一版没有代码变更**，`fundeploy` 的行为和 0.3.6 完全一样，不用急着升。

它存在的唯一原因是发版流程本身换了驱动方式：以前改版本号、同步 `Cargo.lock`、打 tag
这几步是手工做的，现在交给 [funbuild](https://pypi.org/project/funbuild/) 1.6.101
新增的 `CargoBuild` ——它会按 128 进制递增 `Cargo.toml` 的版本、用
`cargo update --workspace` 让 `Cargo.lock` 跟上（已锁定的外部依赖一个都不升级）、
跑 `cargo fmt --all --check` / `cargo clippy --all-targets -- -D warnings` /
`cargo test` 三道关卡，全过了才提交、打 `v*` tag 并推送。发布本身仍然全部由这个 tag
触发的 workflow 完成，`cargo publish` 的位置也没变。这一版就是那条链路的第一次实跑。

## [0.3.6] - 2026-10-15

### 变更

- **`stop` 停不掉的端口现在会腾出来，不再只留一条让人自己敲的 `kill -TERM`。** 以前
  停完端口还被占着时只打一行警告就往下走，而 `start` 遇到端口被占会跳过那个组件——
  于是 `stop` 和 `start` 都报成功、跑的却还是旧进程，`restart` 卡住多半就是这么来的。
  现在按端口反查出占用者，问一声再强停（SIGTERM，5 秒不走上 SIGKILL），停完复查端口
  而不是复查 PID（进程没了不等于端口腾出来了，同一个端口上还可能压着别的用户的进程）。

  ```
  警告: new-api 的端口 8801 停完还占着，占用者是 PID 2593874（python3）
  ? 强制停掉它，腾出端口 8801 吗 (Y/n)
  ```

  这一问刻意跟卸载那种确认反着来：答「不」不算错误、默认选项是「是」（回车就停）、
  **没有终端时不问直接停**。最后一条是关键——cron 和 CI 里要是停下来等输入或者报错
  退出，`restart` 又断在半路，而那正是加这道确认要避免的事。盯着终端但不想每次按
  回车的，设 `FUNDEPLOY_STOP_KILL_PORT_YES=1`。

  `init`（PID ≤ 1）和 fundeploy 自己这两个 PID 永不发信号：杀掉自己之后，复查端口、
  停下一个进程、把服务起回来，一件都不会发生，`restart` 反而断得更彻底。查不到 PID
  的（占用者不属于当前用户）停不动，提示加 sudo 重跑。

  三族服务（`daemon` / `venv` / `upstream`）的 `stop` 共用这个收尾点；`paperclip` 在
  官方 `service stop` 失败后也会先按端口强停一次再判定成败，但不去碰服务管理器里那个
  PID——它有 KeepAlive 守着，杀掉只会被立刻拉起来，把「停不下来」变成更难看的
  「停了又起」。`venv` 的 `stop-one` 原来没有这步收尾，是唯一一条端口没腾出来也不
  吭声的路径，一并补上。

  一次 `stop` 只问一次：`daemon` / `venv` 在 PID 文件丢失时原本自己也会按端口动手，
  那一步现在只负责把归属交代清楚（「它跑的不是本服务的二进制」「它不是这个 venv 里的
  东西」这种话只有那儿答得上来），动手统一归收尾那一处。

  代价说在明处：端口上坐着的**不是**本服务的进程（同机另装了一份、端口号撞了）现在
  也会进入这一问，原先这种情况是刻意不碰的。

## [0.3.5] - 2026-10-15

### 新增

- **`service paperclip prefetch`：把那段下载挪到装之前，也可以挪到后台。**
  managed install 是一条前台命令、不能中断，而它最久的一段是下载——实测冷缓存要下
  431 个 tarball、合计 0.97 GiB（落盘 2.7 GiB、7 万多个文件）。现在可以先把这些灌进
  npm 缓存，等真要装的时候只剩解包：

  ```
  fundeploy service paperclip prefetch canary --background
  ```

  做得到是因为 npm 的缓存按 tarball 的完整 URL 内容寻址，谁灌进去的都算：`npm cache
  add` 之后再装同一个包，npm 在同一个键上命中、一个字节都不再下（日志里是
  `npm http cache … 0ms (cache hit)`）。也正因为键里带着主机名，预热必须跟 install
  走同一个源，换镜像灌的那份对上游那条命令等于不存在。

  清单从 `npm install --package-lock-only` 解析出来的 lockfile 里挑，三道筛子少一道
  都会多下或下错：没有 `resolved` 的是捆在父包 tarball 里的（888 个条目里有 348 个，
  全在 `@paperclipai/server` 肚子里），`os` / `cpu` / `libc` 不匹配的跳过（100 个，
  esbuild 一家就占 25 个平台），`resolved` 不在钉死的源上的只报不灌。别名依赖按
  `name` 字段的真名灌——那 12 个条目恰好是最大的几个（codex 一家 141 + 155 MiB），
  按目录名去灌会 404。

  本机实测：首次 422 个包、缓存涨 908.6 MiB；立刻再跑一次只涨 24 MiB（元数据刷新），
  下载段已经空了。后台那条路有 PID 文件守着不重复开工、跑完自己收掉，进度每批一行
  落进 `~/.paperclip/run/<实例>/prefetch.log`。

- **装 / 升那几分钟不再是一片空白。** 上游把 npm 的输出整个捕获掉了，从
  「Installing paperclipai@x」到装完之间一个字都不打，而那段实测要写 2.5 GiB、7 万多
  个文件，看起来跟卡死一模一样。现在盯着它正在写的那个暂存目录报进度，并且分两段：
  先是下载（暂存目录建好但空着，改看 npm 缓存的增量，没有分母所以不报 ETA），再是
  解包（有分母，这时候 ETA 才有意义，分母按当前 payload 估、估小了就顶上去）。
  stderr 不是终端就原样退回以前的行为，不为一个没人看的进度条每两秒走一遍七万个文件。

### 变更

- **装 / 升之前那条体积提示，口径改对了。** 它以前只报落盘量（「约 2G」），读起来
  像是每次升级都要下 2.7 GiB——实际下载和落盘差了近三倍。现在两个数分开报，下载量按
  实测比值折算（绝对值仍然不写死，依赖树一直在长），并且现场量一眼 `_cacache` 说清楚
  这次是全量还是增量：缓存空着就直说全量，热了就提一句 `PAPERCLIP_NPM_PREFER_OFFLINE=1`
  连校验请求也能省。顺带点明下载量的七成是各家 agent CLI 的二进制（codex /
  claude-agent-sdk / copilot / opencode，而且每个都有两份——上游的版本范围对不上，
  同一个工具两个版本并存），它们升级时大多不变，所以别清 `~/.npm/_cacache`。
- `check` 发现有新版时会多指一条路：先 prefetch 把下载跑完，再 upgrade。
- prefetch 解析依赖图那一步显式放开了 npm 的 `--allow-remote`。npm 12 新增了这条策略
  且默认 `none`，而 `@paperclipai/server` 带 `bundleDependencies`，arborist 为了读出
  里面捆了哪些包必须先把它的 tarball 解开，这一跳会被当成 "remote" 取回拒掉
  （`EALLOWREMOTE`），整条解析挂掉。放开的前提是取回范围另有闸门管着：逐条核对
  `resolved` 的主机名，不在钉死的源上的一个都不碰。注意同一条策略也会让上游那条
  `npm install` 在 npm 12 上直接失败，那个还没处理——装不上的时候先看 `npm --version`。

## [0.3.4] - 2026-10-15

### 新增

- **`upgrade` 现在会把升级交回原渠道，不再只是把命令打印出来让人自己敲。** 0.3.1 起
  包管理器装的那份一律拒绝升级（原地换文件那边不知情，下一次 `brew upgrade` 会盖回
  去），拒绝的判断没错，但拒绝完撒手不管，等于同一条 `fundeploy upgrade` 在一半的
  装法下都不干活。现在它先认出这份二进制归谁管，再走对应的路：

  | 装法 | 做什么 |
  | --- | --- |
  | 装机脚本 / 直接下二进制 | 照旧：下 tar.gz，原子替换掉自己 |
  | Homebrew | 进 tap 仓库 `git pull --ff-only`，再 `brew upgrade <tap>/fundeploy` |
  | APT | `apt-get update` 后 `apt-get install fundeploy`（按需加 sudo） |
  | cargo | `cargo install fundeploy --force --locked` |

  Homebrew 那条路两步缺一不可：tap 是个普通的 git clone，不刷新的话里面的 formula
  还写着旧版本，`brew upgrade` 自然「无事可做」而且退出 0——看着像升过了，其实没动。
  tap 目录从 keg 的 `INSTALL_RECEIPT.json` 里的 `source.tap` 问 `brew --repository`
  要（自己拼 `Library/Taps/...` 容易拼错），问不出来就退到从 `source.path` 往上找
  `.git`，再找不到就把刷新交给 brew 自己的 auto-update 并说一声。tap 刷成功了才加
  `HOMEBREW_NO_AUTO_UPDATE=1`：那一步要去 GitHub 把所有 tap 拉一轮，国内就是干等，
  而我们要的那个 formula 已经是最新的了。

- **交回渠道之后会核实再下结论。** `brew upgrade` / `apt-get install` 在「没有更新
  可升」时同样退出 0，所以升完再问一次那边现在装着哪一版（brew 问
  `brew list --versions`，cargo / apt 问那个文件自己的 `--version`——它俩的路径升级
  前后不变）：真新了就说新了；还是同一版就明说那边暂时没有更新的，并提醒发版后 apt
  索引、formula、crates.io 都要等几分钟；装出个更旧的、或者 `--version` 点的和落地的
  不是一版，都报警告而不是成功。

### 变更

- `upgrade --version` 的边界说清楚了：brew 的 tap 里只有一个跟着最新版走的 formula，
  apt 源也只放最新一版，这两种安装下点版本直接报错，并给出走得通的路（装一份到
  `~/.local/bin` 自己管，那条路上点版本一直好使）。cargo 装的照旧支持
  （`cargo install --version`）。
- `upgrade --check` 会多报一行安装渠道，顺带说明这种安装下升级背后会跑什么命令。

## [0.3.3] - 2026-10-14

### 修复

- **`service paperclip stop` 不再把「本来就停着」报成失败。** 服务没在跑的时候停一
  次，得到的是：

  ```
  Command failed: launchctl bootout gui/501/ing.paperclip.paperclipai
  Boot-out failed: 3: No such process

  错误: paperclipai service stop 失败（退出码 1）
  ```

  根子在官方 CLI：它 launchd 后端的 `stop()` 是一句裸的 `launchctl bootout`，job 没
  加载时 bootout 以 ESRCH(3) 退出。同一个 bootout 在它的 install / uninstall 里都带
  着 `.catch()` 吞掉，偏偏 stop 这一处没有（systemd 后端的 `systemctl --user stop`
  对没加载的单元同样非零退出）。于是一个本该是 no-op 的操作成了硬失败。

  现在官方 stop 报错之后自己核实一遍再下结论：服务管理器里没有 pid、job 也不是「加
  载着、进程没了会被 KeepAlive 拉回来」的状态、端口还没人在听——三条都成立就按已停
  止收场，并说明那条报错为什么不改变结果。任何一条不成立照旧报失败：真停不下来
  （没权限、卡在退出中、马上会被拉起来）必须保留失败语义，这种地方的假成功比报错
  更坏。

  顺带，官方 stop 失败时也会去收 fundeploy 托管的那个后台进程。失败的常见原因恰恰
  是服务管理器里根本没有这个服务，那种机器上真在跑的就是托管的那一个。

## [0.3.2] - 2026-10-14

### 新增

- **GitHub 连不上时改用 Gitee 镜像，装机和升级都支持。** 镜像一直在同步（CI 把
  整个仓库连 release 附件一起搬过去），但装的时候用不上——地址是写死的 GitHub，
  国内连不上就只能干看着。现在装机脚本和 `fundeploy upgrade` 都先试 GitHub、
  不通再换 Gitee，换的时候会出声说一句，不会让人不知道这份是从哪儿下的。

  `FUNDEPLOY_RELEASE_SOURCE=github|gitee` 可以点名。点了名就只用那一个，失败直接
  报错——要确定性的人不该被背着换源。嫌默认那次探测慢的直接点 `gitee`。

  脚本本身也能从 Gitee 取，这样连 `raw.githubusercontent.com` 都不通也装得上：

  ```bash
  curl -fsSL https://gitee.com/farfarfun/fundeploy/raw/master/install.sh | sh
  ```

  几处落地上的取舍：探测 GitHub 那一下把建连超时压到 8 秒并关掉重试，否则
  「连不上就换一个」要先让人等三轮二十多秒；加速前缀只往 GitHub 的地址上套，
  拿去绕 Gitee 只会把一个本来通的地址弄坏，而设了那个前缀的人恰恰就是会走到
  Gitee 这条路上的人；校验和跟着包走同一个源，不会从 A 下包拿 B 的清单。

  只管 fundeploy 自己。装别的工具取的是上游仓库的 release，那些没有镜像，照旧
  直连 GitHub。

- **`brew install` 同样有了 Gitee 退路。** formula 里每条 `url` 后面跟一条 Gitee
  `mirror`（brew 原生支持，下包那一步它自己回落，不用配任何东西）；`brew tap` 做的
  是 git clone，没法在 formula 里回落，所以把 tap 仓库也镜像了一份：

  ```bash
  brew tap farfarfun/packages https://gitee.com/farfarfun/packages
  brew trust farfarfun/packages
  brew install fundeploy
  ```

  apt 没有对应的路。deb 树是 Actions 部署到 GitHub Pages 的，不在 git 里，镜像
  仓库带不走它——`farfarfun.github.io` 连不上就用装机脚本。

## [0.3.1] - 2026-10-13

### 修复

- `upgrade` 不再悄悄覆盖包管理器装的那份。此前只检查目录可不可写，而
  `~/.cargo/bin` 和 Apple Silicon 上的 `/opt/homebrew/bin` **对当前用户就是可写
  的**——于是替换会成功，而包管理器那边毫不知情：`brew list --versions` 报的还是
  它自己装的那一版，下一次 `brew upgrade` 又把我们换上去的盖回去，两边来回覆盖，
  全程没有一句报错。现在认出 cargo / Homebrew / dpkg 装的二进制就拒绝，并告诉你
  该敲哪条命令升（`brew upgrade fundeploy` 等），想要一份自己说了算的就装到
  `~/.local/bin`。

### 新增

- 发到 [crates.io](https://crates.io/crates/fundeploy)，`cargo install fundeploy`
  可装。这是给手上已经有 Rust 工具链的人的次要渠道——它从源码编，而这个工具本身
  是用来装工具链的，当主路径顺序就反了。

### 变更

- **分发全部收到 [farfarfun/packages](https://github.com/farfarfun/packages)。**
  原来每个项目各发一个 apt 源，装 n 个包要加 n 次源。现在 farfarfun 所有项目共用
  一个仓库，它同时是 apt 源和 Homebrew tap，两边的索引由同一张 `packages.toml`
  生成：

  ```bash
  # APT
  curl -fsSL https://farfarfun.github.io/packages/farfarfun.gpg \
    | sudo tee /usr/share/keyrings/farfarfun.gpg >/dev/null
  echo "deb [signed-by=/usr/share/keyrings/farfarfun.gpg] https://farfarfun.github.io/packages/deb stable main" \
    | sudo tee /etc/apt/sources.list.d/farfarfun.list

  # Homebrew
  brew tap farfarfun/packages https://github.com/farfarfun/packages
  brew trust farfarfun/packages
  ```

  加过之后新包直接 `apt install` / `brew install`，裸名字即可。

  `tap` 那行得带 URL：`brew tap a/b` 只认 `github.com/a/homebrew-b` 这个约定，而这
  个仓库同时是 apt 源，叫 `homebrew-packages` 的话 apt 地址里会凭空多出几个字。

  `trust` 那行是 Homebrew 6.0 起的新要求：第三方 tap 默认不可信，不信任的话
  `brew install fundeploy` 直接被拒（`Refusing to load formula ... from untrusted
  tap`）。写全名 `brew install farfarfun/packages/fundeploy` 能装是因为它顺带只信任
  了那一个 formula，所以这个坑很容易被误读成「裸名字不好使」。

- 本仓库的 `packaging/` 相应只剩 `build-deb.sh`。

## [0.3.0] - 2026-10-13

整个项目从 Bash 重写为 Rust，发布物从一棵脚本树变成单个静态二进制。原来的
Bash 保留在 `bak` 分支，`master` 上的 `scripts/` 与 `tests/` 已整体删除。

这一节只记录**行为上的差异**——命令、环境变量、默认端口跟 Bash 版对不上的
地方；内部怎么分层、代码少了多少行，用不着写在这里。

### 破坏性变更

- 默认端口不再撞车。Bash 时代 `8806` 被三个服务同时当默认值用（funfluid-web
  前端、airflow 的 Web UI、celery 的 Flower），`8812`/`8813` 被 funmill 和
  funlesson-web 共用——每个服务一个脚本，谁也看不见别人，于是「同机起两个服务，
  第二个莫名其妙起不来」。现在所有服务的端口在同一张表里，撞了就是一条编译期
  就会红的测试。挪的是后来者，原位留给挪动代价最大的那个：

  | 服务 | 端口 | 原默认 | 现默认 | 覆盖用 |
  |---|---|---|---|---|
  | `airflow` | Web UI | 8806 | **8815** | `AIRFLOW_PORT` / `AIRFLOW__API__PORT` |
  | `celery` | Flower | 8806 | **8816** | `FLOWER_PORT` |
  | `funlesson-web` | 前端 | 8812 | **8814** | `FUNLESSON_WEB_FRONTEND_PORT` |

- `celery`：三个依赖规格环境变量改名，语义也收窄了——`CELERY_PKG_SPEC` →
  `CELERY_VERSION`（只填版本号 `5.4.0`，不再是整串 `celery==5.4.0`），
  `CELERY_REDIS_PKG_SPEC` → `CELERY_REDIS_SPEC`，`CELERY_FLOWER_PKG_SPEC` →
  `CELERY_FLOWER_SPEC`（后两个仍是完整 spec）。
- `celery`：按组件分裂的 `start-worker` / `start-beat` / `start-flower` /
  `run-worker` / `run-beat` / `run-flower` 换成三个通用动作——
  `start-one <进程名>`、`run-one <进程名>`、`stop-one <进程名>`，进程名就是
  `worker` / `beat` / `flower`。
- `open-pencil` 从服务挪到工具：`fundeploy service open-pencil …` →
  **`fundeploy tool open-pencil …`**。它装完是两个命令行程序，其中
  `openpencil-mcp` 由 MCP 客户端按需拉起、用完即走，没有一个归 fundeploy 管的
  常驻进程——留在服务里就得有四个撒谎的动作（`start` 报成功而什么都没起，
  `status` 答「已停止」而它压根不是跑的东西）。动作相应收敛为
  `install` / `upgrade` / `status` / `uninstall` + `install-desktop` /
  `mcp-config`。环境变量 `OPEN_PENCIL_SERVICE_HOME` 随之改名
  `OPEN_PENCIL_HOME`（旧名仍然认，机器上已下载的 `dist/` 不会因此找不到）。
- `cockpit-tools` 的子命令从 `install` / `update` / `reinstall` / `uninstall` /
  `run` 收敛为 `install` / `upgrade` / `status` / `uninstall` + `run`。
  `reinstall` 没了——它做的事就是删掉再 `install`，而 `install` 本来就是覆盖写。
  `COCKPIT_TOOLS_FALLBACK_TAG` 也没了，理由见下面「修复」。
- 工具的卸载确认开关统一成 `FUNDEPLOY_<工具名>_UNINSTALL_YES`：
  `BREW_UNINSTALL_YES` → `FUNDEPLOY_BREW_UNINSTALL_YES`，
  `COCKPIT_TOOLS_UNINSTALL_YES` → `FUNDEPLOY_COCKPIT_TOOLS_UNINSTALL_YES`，
  `OPEN_PENCIL_UNINSTALL_YES` → `FUNDEPLOY_OPEN_PENCIL_UNINSTALL_YES`。带前缀是
  为了跟上游自己的环境变量分开——`BREW_*` 那一串名字的归属是 Homebrew，不是这里。
- `skills`（原 `skills-sync`）的环境变量统一加 `FUNDEPLOY_` 前缀：
  `SKILLS_SYNC_DIR` → `FUNDEPLOY_SKILLS_DIR`，`SKILLS_SYNC_ORG` →
  `FUNDEPLOY_SKILLS_ORG`，`SKILLS_SYNC_API_URL` → `FUNDEPLOY_SKILLS_API`。
  目标目录的默认值也跟着组织名走了（`~/workspace/github/<组织名>`，原先无论
  指到哪个组织都写死 `farfarfun-skills`）——换组织时两边的仓库不再堆在同一个
  目录里按名字互相覆盖。
- `skills` 的动作重排：原先的 `list`/`status` 是同一件事，`update`/`upgrade`、
  `delete`/`remove`/`uninstall` 各是三个别名；现在是
  `list` / `sync` / `install <名字>` / `update <名字>` / `rm <名字>|--all`。
  `sync` 是新的主力动作——一条命令把缺的装上、已有的更到最新，原先得先
  `install` 再 `update` 跑两遍。
- `python-env` 从工具挪到开发工具链域：`fundeploy tool python-env …` →
  **`fundeploy dev python …`**。它跟 go/nodejs/rust/uv 一样是「给这台机器准备
  开发环境」，跟 brew/cockpit-tools 那种装一个应用不是一回事。
  `reinstall` 换成 `install --force`——两个动作差的只是「先删不先删」，而
  `reinstall` 这个名字看不出它会把人后来往环境里装的别的包一起扔掉。
- `dev` 域的装法不再只能靠环境变量：`INSTALL_METHOD` →
  **`--method pkg|source`**（环境变量仍可用，改名 `FUNDEPLOY_INSTALL_METHOD`，
  命令行优先）。卸载时不点名仍然是两边都清。
- `dev` 域各工具的 `version` 子命令并进 `status`。只打一个版本号回答不了真正
  会出问题的那个局面——机器上同时存在包管理器装的和 `~/opt` 下装的两份，
  `version` 报的是 PATH 上那份，而 `upgrade` 动的是 `~/opt` 那份，于是「升完
  版本号没变」。`status` 两份都列，版本对不上时明说该怎么调 PATH。
- `dev` 域的环境变量统一加 `FUNDEPLOY_` 前缀，跟上游自己的变量分开——
  `UV_INSTALL_DIR` 这类名字的归属是 uv 的安装器，不是这里：

  | 原名 | 现名 |
  |---|---|
  | `INSTALL_METHOD` | `FUNDEPLOY_INSTALL_METHOD` |
  | `GO_INSTALL_ROOT` | `FUNDEPLOY_GO_ROOT` |
  | `NODE_INSTALL_ROOT` | `FUNDEPLOY_NODE_ROOT` |
  | `RUST_INSTALL_ROOT` | `FUNDEPLOY_RUST_ROOT` |
  | `UV_INSTALL_DIR` | `FUNDEPLOY_UV_ROOT` |
  | `UV_INSTALL_URL` | `FUNDEPLOY_UV_INSTALL_URL` |
  | `NPM_GLOBAL_ROOT` | `FUNDEPLOY_NPM_GLOBAL_ROOT` |
  | `PNPM_USE_NPM_GLOBAL` | `FUNDEPLOY_PNPM_VIA_NPM` |
  | `PYTHON_VERSION` | `FUNDEPLOY_PYTHON_VERSION` |
  | `PYTHON_ENV_ASSUME_YES` | `FUNDEPLOY_PYTHON_UNINSTALL_YES` |
- `nodejs` 不再接受 `NODE_VERSION` 指定版本，改用通用的 `--version`。
  `NODE_VERSION` 是 Node 生态里被 nvm、CI 镜像、Dockerfile 共用的名字，
  在这里额外赋一层意思只会让人在已经设了它的机器上装到意料之外的版本。
- `ai`（原 `ai-cli`）的装法从位置参数改成选项：`fundeploy ai claude pnpm install`
  → **`fundeploy ai claude install --method pnpm`**。位置参数那种写法要靠「第一个
  词是不是已知装法」来猜它到底是装法还是动作，猜错时报的话也是错的（见下面
  「修复」）。
- `ai` 的动作别名收敛成一个名字：`update`/`upgrade` → `upgrade`，
  `remove`/`uninstall` → `uninstall`，`version`/`status` → `status`。
- `ai list` 没了。`fundeploy ai` 不带参数现在直接列三样工具的状态（装没装、
  装在哪儿、哪个版本、有哪些装法可走），比一串光秃秃的名字有用。
- `ai` 的环境变量分两类，各按归属命名：

  | 原名 | 现名 | 谁读它 |
  |---|---|---|
  | `CLAUDE_INSTALL_URL` | `FUNDEPLOY_CLAUDE_INSTALL_URL` | fundeploy |
  | `CODEX_INSTALL_URL` | `FUNDEPLOY_CODEX_INSTALL_URL` | fundeploy |
  | `CURSOR_INSTALL_URL` | `FUNDEPLOY_CURSOR_INSTALL_URL` | fundeploy |
  | `FUNDEPLOY_ASSUME_YES` | `FUNDEPLOY_<工具名>_UNINSTALL_YES` | fundeploy |
  | `CODEX_INSTALL_DIR` | 不变 | **上游安装器** |
  | `CODEX_HOME` | 不变 | **上游安装器** |

  后两个保持原样不是漏改：卸载时要按跟安装器完全相同的规则算出它把文件放到了
  哪儿，改了名就算不出来，卸载会静默漏掉东西。原来那个
  `FUNDEPLOY_ASSUME_YES` 则是全域共用一个开关，设一次等于对所有工具的删除都
  点了头。

- **装机方式变了：下一个二进制，不再 clone 仓库。** fundeploy 现在是单个静态
  二进制，装机要做的只剩「把本机对应的那一个放到 PATH 上」：

  ```bash
  curl -fsSL https://raw.githubusercontent.com/farfarfun/fundeploy/HEAD/install.sh | sh
  ```

  随之变化的：

  | 原先 | 现在 |
  |---|---|
  | `./install.sh install\|update\|uninstall` | `sh install.sh`（只有装这一件事） |
  | 装到 `~/.local/fundeploy/{bin,src}` | 装到 `~/.local/bin/fundeploy` 一个文件 |
  | `--source github\|gitee` | 没有了，见下 |
  | 自动往 `~/.bashrc` 写 PATH | 只打印该加的那一行 |
  | `fundeploy upgrade`（重跑装机脚本） | `fundeploy upgrade`（见「新增」） |
  | 卸载要跑脚本 | 删掉那个文件就行 |

  `--source gitee` 去掉是因为它已经是个空承诺：Gitee 上的镜像仓库一个 release
  都没有，这条路取不到任何东西。国内网络改走通用的镜像前缀
  `FUNDEPLOY_GITHUB_HUB_PROXY_PREFIX`——那套机制本来就在，`fundeploy doctor
  github` 也认它，比「换个仓库」覆盖面更大。

  不再写 shell 配置，是因为偷偷往 `~/.bashrc` 里塞东西意味着卸载时得把它找回来
  删掉，而卸载这件事现在只是 `rm`，没有地方做这个清理。

- 装机脚本的环境变量跟着换了一批：

  | 原名 | 现在 |
  |---|---|
  | `FUNDEPLOY_ROOT` / `FUNDEPLOY_WRAPPER_ROOT` / `FUNDEPLOY_SRC_DIR` | `FUNDEPLOY_BIN_DIR`（默认 `~/.local/bin`） |
  | `FUNDEPLOY_GITHUB_REPO` / `FUNDEPLOY_GITEE_REPO` | `FUNDEPLOY_RELEASE_REPO` |
  | `FUNDEPLOY_GIT_CLONE_REF` | `FUNDEPLOY_VERSION`（或 `--version`） |
  | `FUNDEPLOY_SKIP_GIT_PULL` | 没有了（不再有本地仓库） |
  | `FUNDEPLOY_SKIP_PROFILE_HINT` | 没有了（本来就不写配置了） |
  | `FUNDEPLOY_UNINSTALL_YES` | 没有了（卸载就是 `rm`） |

- `.deb` 从 `Architecture: all` 改成每个架构一个包
  （`fundeploy_<版本>_amd64.deb` / `_arm64.deb`），`Depends` 去掉只留
  `Recommends: ca-certificates, git, curl`——二进制是静态链接的，装上就能跑，
  那几个只有个别子命令要用。
- Homebrew formula 从「下源码」改成装预编好的二进制。从源码编要先有 Rust 工具链，
  而 fundeploy 本身就是用来装工具链的，这个顺序反了。
- 根目录的 `VERSION` 文件没了，版本号只剩 `Cargo.toml` 一个来源——二进制自报的
  版本来自 `CARGO_PKG_VERSION`，两处分开写迟早会对不上，而对不上的表现是
  `upgrade` 每次都认为有新版可升。

### 新增

- 「包 pip 装进独占 venv、进程归 fundeploy 管」的服务（目前是 `airflow`、
  `celery`）统一支持 `start-one` / `run-one` / `stop-one`，不再是 celery 专属。
- `fundeploy tool`：工具域。跟服务的分界线只有一条——工具没有常驻进程，所以
  动作只有装、升、卸、查。`fundeploy list` 现在同时列出服务和工具。目前有
  `brew`、`cockpit-tools`、`open-pencil`。
- `cockpit-tools`、`brew` 都多了 `status`：装没装、装在哪儿、哪个版本。Bash 版
  只有 brew 有个 `version` 子命令，cockpit-tools 则无从查起。
- `cockpit-tools status` 会指出「桌面入口还在、它指向的 AppImage 没了」这种悬空
  状态——在应用菜单里点它既不启动也不报错，光看菜单看不出东西已经不在了。
- `cockpit-tools install` 现在校验下载下来的 AppImage：上游的清单叫
  `SHA256SUMS.txt`，而此前只认 `checksums.txt` 这一个名字，于是校验一直被跳过。

- **`fundeploy source`：换源域。** 五个包管理器一套命令：

  ```
  fundeploy source                        # 五个的当前状态一次看全
  fundeploy source pip list               # 有哪些源
  fundeploy source pip test               # 并发测延迟，快的在前
  fundeploy source pip use                # 不给名字＝自动换到实测最快的
  fundeploy source pip use tuna           # 点名
  fundeploy source pip use https://内网/simple/   # 直接给地址
  fundeploy source pip add corp https://nexus.corp/simple/ --label 公司源
  fundeploy source pip rm corp
  fundeploy source pip reset              # 恢复官方源
  ```

  覆盖 `pip`（pip/uv/poetry/pdm 共用的 `pip.conf`）、`npm`（`.npmrc`，
  pnpm/yarn/bun 共用）、`brew`、`cargo`（`sparse+` 索引）、`go`
  （`GOPROXY`+`GOSUMDB`）。自己加的源存在
  `~/.config/fundeploy/sources.toml`，同名时盖掉内置的——镜像站改了路径不必
  等 fundeploy 发版。`FUNDEPLOY_SOURCE_PROBE_TIMEOUT`（秒，默认 5）和
  `FUNDEPLOY_SOURCE_PROBE_JOBS`（默认 8）调测速行为。

  几处跟社区常见做法不同、但都是踩过的坑：

  - **brew 换的是三个变量，不是一个。** 网上绝大多数教程只给
    `HOMEBREW_BOTTLE_DOMAIN`，那只管二进制包下载；配方元数据走
    `HOMEBREW_API_DOMAIN`，brew 自身更新走 `HOMEBREW_BREW_GIT_REMOTE`。
    少换一条的症状就是「我明明换了源，`brew install` 开头还是卡住」。
  - **go 同时写 `GOSUMDB`。** `sum.golang.org` 在国内不通，而它不走 GOPROXY。
    只换 GOPROXY 的表现是下载飞快、然后卡在
    `verifying module: … dial tcp: i/o timeout`，看起来像代理没生效。
  - **写进别人的配置文件时用带首尾标记的区块**（`# >>> fundeploy … >>>`），
    改是整段替换，删只删自己那段。社区脚本惯用的「追加 export + 按关键字
    `sed -i /关键字/d`」换三次源就留三份互相覆盖的 export，而按关键字删行会
    连带删掉任何恰好提到那个词的行（包括注释和别人的配置）。
  - **改 pip.conf / `.npmrc` / go env 是按行改**，不是整份解析再写回——后者
    语义等价但人写的注释、分节顺序全没了。
  - **cargo 发现 `~/.cargo/config.toml` 里已经有 `[source.crates-io]` 就报错
    停下**，不硬写。TOML 不允许同名表出现两次，写进去的后果是 cargo 连
    `--version` 都跑不起来。
  - **brew 只写当前用户的 shell 配置**（zsh → `~/.zprofile`，bash →
    `~/.bash_profile` 或 `~/.profile`），不碰 `/etc/profile`。
  - **测速量的是首字节时间**，不下完整个文件——brew 的 `formula.jws.json` 有
    34 MB，真下一遍会把「测速」变成「下载」。量的也不是站点首页，而是各
    协议里必定存在的小入口（PyPI 的 `setuptools/` 索引页、npm 的 `npm` 包
    元数据、cargo 的 `config.json`、go 的 `@v/list`），否则量到的是「404 回得
    有多快」。

  内置的 30 多个镜像地址都在加进来之前实测过，顺手剔掉了三个 HomebrewCN /
  旧教程里仍在流传但实际已经不能用的：上海交大的 Homebrew 镜像（全部路径
  404）、豆瓣的 PyPI 源（已下线）、中科大的 `homebrew-core.git`（Homebrew 4.0
  起默认走 API 不再克隆这个仓库，站点已下线；所以 `HOMEBREW_CORE_GIT_REMOTE`
  是**可选**项，只有手动 tap 过的人才用得上）。

  阿里内网的 `http://yum.tbsite.net/pypi/simple` 和
  `http://pypi.antfin-inc.com/simple` 没有收进内置表——它们只有明文 HTTP，
  在公网也无法验证。需要的话自己加：

  ```
  fundeploy source pip add tbsite http://yum.tbsite.net/pypi/simple --label 阿里内网
  fundeploy source pip add antfin http://pypi.antfin-inc.com/simple --label 蚂蚁内网
  ```

  换到明文 HTTP 源时会警告一次，并自动补上 pip 要求的 `trusted-host`；换回
  https 源时那行会被清掉（留着等于对那台主机永久关掉校验）。

- `fundeploy tool brew install` 会带上已配好的 brew 镜像再跑官方安装器。官方
  `install.sh` 自 2022 年起就认 `HOMEBREW_BREW_GIT_REMOTE` 这组变量，所以第一次
  装就能走镜像——没配过的话它会去 GitHub 克隆几百 MB 的 brew 仓库，这正是国内
  装 brew 卡住的那一步。（HomebrewCN 的做法是把 install.sh 下载下来用 `sed` 把
  里面的 GitHub 地址逐条换掉，上游改一行就断一次。）

- **`fundeploy port-kill <端口>…`：腾端口。** 原 `fundeploy port-kill` 工具，
  提到顶层命令。

  ```bash
  fundeploy port-kill --list 8080 8081   # 只看谁占着
  fundeploy port-kill 8080               # 先 SIGTERM，15 秒不退再 SIGKILL
  fundeploy port-kill -y 8080            # 不问直接停（或设 FUNDEPLOY_PORT_KILL_YES=1）
  ```

  跟 Bash 版的几处不同：

  - **占用者不再只报一个。** Bash 版拿 `lsof -ti` 的第一行，而 `fork` 出来的
    worker 继承着同一个监听 fd——gunicorn、nginx 都是这样，只停 master，worker
    还攥着端口不放，结果就是「说停好了，端口还是占着」。现在同一个端口上的进程
    全列出来、全停。
  - **不再把出站连接算成占用。** `lsof -i :8080` 匹配的是本地**或**远端端口，
    本机一个临时端口恰好是 8080 的出站连接也会被它算进来；照着那个结果去 kill，
    杀掉的是一个跟这个端口毫无关系的进程。现在只认监听中的 TCP 和已绑定的 UDP。
  - **复查的是端口，不是 PID。** 非 root 时读不到别人的 `/proc/<pid>/fd`
    （`ss -p` 同样打不出 pid），「看不见占用者」会被当成「端口空了」报成功。
    现在停完之后另查一次端口本身，还占着就明确报出来并提示加 `sudo`。
  - 自己和 1 号进程进保护名单，跳过并说明原因，不会把 init 或 fundeploy 自身停掉。
  - 确认那一步走统一的 `ui::confirm_destructive`：无人值守下不给
    `FUNDEPLOY_PORT_KILL_YES=1` 就直接报错退出，而不是默认同意。

  Linux 上不依赖 `lsof`/`ss`（容器镜像里常常两个都没装），直接读
  `/proc/net/{tcp,tcp6,udp,udp6}`；macOS 退回 `lsof`。

- **`fundeploy doctor github`：GitHub 连不上的诊断与修复。** 原
  `fundeploy github-net`，进 `doctor` 域。

  ```bash
  fundeploy doctor github                  # 只诊断，不改任何配置
  fundeploy doctor github --fix            # 按诊断结果自动挑一条改过去
  fundeploy doctor github --fix ssh443     # 指定通道（ssh22 / ssh443 / https）
  fundeploy doctor github --undo           # 撤掉写进 git / ssh 的东西
  ```

  跟 Bash 版的几处不同：

  - **结论从两档变成四档。** Bash 版每条通道只报「通/不通」，于是「22 端口被
    防火墙丢包」和「SSH key 还没传到 GitHub」长得一模一样——而后者换到哪条通道
    都一样不行。现在分成「通」「端口通但缺 key」「端口通但本机没有它的主机
    密钥」「不通」，后两种各给各的下一步。
  - **`--fix` 换通道时会把上一条的规则清掉。** 见下面「修复」。
  - **不再写 `IdentitiesOnly yes`。** Bash 版往 `~/.ssh/config` 里写的 443 区块
    带这一行，而区块里并没有配套的 `IdentityFile`：ssh 于是只认配置里点名的
    密钥，把 ssh-agent 里的全部忽略掉。用 1Password / YubiKey 托管密钥的人一改
    完就变成 `Permission denied (publickey)`，且看不出跟这条命令有关。
  - **主机密钥不再现场抓。** Bash 版 `ssh-keyscan` 抓一份、算指纹、跟写死的值
    比对、相符才写。现在直接写死那把公钥本身（`github.com:22` 和
    `ssh.github.com:443` 用的是同一把）——比对指纹和比对公钥是同一件事，而少一次
    联网就少一个「这条命令只在网络有问题时才跑」的自相矛盾。已经记着另一把
    密钥时不覆盖，只提示 `ssh-keygen -R`。
  - **`--undo` 真的会撤销。** Bash 版的 `uninstall` 只是把手工步骤打印出来。
  - **三条通道并行探，整条命令 12 秒封顶。** Bash 版串行跑，三条都卡住就是
    三倍等待；`git ls-remote` 还没有连接超时可配，碰上丢包型防火墙能耗两分钟。

- **`fundeploy skills`：技能仓库同步。** 把 Gitee 组织里名字带 `--` 的仓库同步
  到本地一个目录，每个仓库一个子目录；同步完把除 `.git` 之外的点目录递归删掉
  （`.github`、`.cursor`、`.claude` 装的是那个仓库自己开发时的配置，跟过来会被
  读技能的工具当成当前环境的配置来用）。

  ```
  fundeploy skills list              # 远端有哪些，本机各是什么状态
  fundeploy skills sync              # 缺的装上，有的更到最新
  fundeploy skills install <名字>
  fundeploy skills update <名字>
  fundeploy skills rm <名字> | --all
  ```

  跟 Bash 版（`scripts/tools/skills-sync/`）的差别：

  - **装上去还删得掉。** 见下面「修复」。
  - **一个仓库失败不带停剩下的。** Bash 版 `_run_many` 碰上出错就整条命令退出，
    而这条命令的价值恰恰在于一次把几十个拉齐——中途断掉等于每次都得从头再来。
    现在逐个记账，末尾汇总「装了几个、更新几个、跳过几个」，有失败才非零退出。
  - **翻页有上限。** Bash 版 `while true` 一直翻到空页为止；Gitee 对越界页码
    偶尔返回最后一页而不是空数组，撞上就是死循环。现在 50 页封顶。
  - **符号链接不会被当成已安装的仓库。** Bash 版用 `-d` 判目录，而 `-d` 会跟随
    符号链接：目标目录里有一条同名链接指向别处，`reset --hard` / 递归删除动的
    就是链接那头的东西。现在用 `lstat`，链接一律算「目录冲突」，三个写操作全部
    停手。清点目录时同理，只删链接本身，不跟进去。
  - **`update` 之后仓库不再越长越大。** Bash 版 `clone --depth 1` 之后
    `git fetch` 不带深度，第一次更新就把整部历史拉了回来。现在 fetch 也带
    `--depth 1`。
  - 远端已经没有、本地还留着的仓库会在 `list` 里标成「仅本地」。Bash 版只列
    远端有的，上游改名或删库之后，本地那份就从视野里消失了，但目录还在。

- **`fundeploy dev`：开发工具链域。** `go`、`nodejs`、`pnpm`、`rust`、`uv` 五样
  工具加上 `python` 虚拟环境，一套命令：

  ```
  fundeploy dev                          # 六样东西的状态一次看全
  fundeploy dev go status
  fundeploy dev go install               # 默认走包管理器
  fundeploy dev go install --method source --version 1.25.0
  fundeploy dev rust upgrade --method source
  fundeploy dev nodejs uninstall         # 不点名装法＝pkg 和 source 两边都清
  fundeploy dev python install --version 3.13
  ```

  包管理器那条路认 apt / dnf / yum / pacman / zypper / apk / brew，每家的包名
  各自声明（Go 在 apt 下叫 `golang-go`、dnf 下叫 `golang`、brew 下叫 `go`）；
  源里根本没有的（Debian 稳定源里的 uv、多数发行版的 pnpm）直接说没有，不去跑
  一条注定报「软件包未找到」的命令。

  跟 Bash 版的几处不同：

  - **`fundeploy dev` 不带参数是一张总表**，六样东西各报「`~/opt` 下那份」和
    「PATH 上那份」两行。Bash 时代每样工具是一个独立脚本，想知道这台机器上
    到底是个什么局面，得挨个跑一遍 `version`。
  - **版本号不写死。** Go 问 `go.dev/VERSION?m=text`，Node 读
    `nodejs.org/dist/index.json` 取最新 **LTS**（不是绝对最新——新的大版本头
    几个月里原生模块编译失败是常态）。
  - **上游安装脚本先下到文件再执行**，不用 `curl … | sh`。管道执行是边下边跑
    的，连接中途断掉时脚本已经执行了前半截，留下一堆既不算装上也不算没装的
    半成品。
  - **uv 装在哪儿是找出来的，不是猜的。** 上游安装器在不同版本里把二进制放过
    `<dir>` 也放过 `<dir>/bin`，猜错了打印出来的 PATH 就是错的，人照着配完会
    发现命令仍旧找不到。
  - **pnpm 走 corepack 装完也认得出来。** corepack 的 shim 落在 Node 的 bin
    里，不在 npm 全局前缀下，两处都查才不会「刚装完就报未安装」。
  - `FUNDEPLOY_PYTHON_ROOT` 和 `FUNDEPLOY_PYTHON_PACKAGES` 是新的：Bash 版的
    环境根目录写死 `~/opt`，预装包写死在一个 bash 数组里。
  - `fundeploy dev python list` 会把不在版本表里、但确实躺在根目录下的 `py*`
    环境也列出来——版本表是会改的，改完之前建的环境不该从视野里消失，而目录
    还占着地方。

- **`fundeploy ai`：AI 命令行工具域。** `claude`、`codex`、`cursor`：

  ```
  fundeploy ai                           # 三样的状态一次看全
  fundeploy ai claude install            # 默认走官方安装器
  fundeploy ai claude install --method npm
  fundeploy ai codex upgrade
  fundeploy ai claude uninstall          # 不点名装法＝四条路都走一遍
  fundeploy ai all install               # 三样都装上
  fundeploy ai cursor install -- stable  # -- 之后的原样交给官方安装器
  ```

  跟 Bash 版的几处不同：

  - **`fundeploy ai` 不带参数是一张总表**，每样报「在 PATH 上的哪儿、哪个版本、
    有哪些装法可走」。Bash 版的 `ai list` 只打三个名字。
  - **`ai all` 碰到走不通的那样不会整个停下。** Bash 版 `ai all brew install`
    走到 cursor 就死了（cursor 只有 official），而前面两样装没装成也不会单独
    告诉你。现在不支持的跳过、失败的记下来接着走，最后一次报齐。
  - **官方安装器先下到文件再执行**，不用 `curl … | bash`，理由同 `dev` 域。
  - **brew 那条路在 Linux 上提前说清楚。** cask 是 macOS 独有的机制，
    Linuxbrew 上 `brew install --cask` 只会换来一句
    `Installing casks is supported only on macOS`，看着像 brew 自己坏了。
  - **npm / pnpm 优先用本域装的 Node。** `fundeploy dev nodejs install
    --method source` 之后 npm 还不在 PATH 上（PATH 要人自己配），此时报
    「未找到 npm」是误导。
  - **额外参数只有 official 认，给别的装法会被拒绝**，不是悄悄丢掉。
  - **卸载完如果 PATH 上还找得到这个命令，会明说**，并指出那份不是 fundeploy
    装的——否则「卸了但命令还在」会被当成卸载失败。
  - `cursor` 的两个命令名（`agent` 和 `cursor-agent`，上游改过名）都认，
    查状态和卸载都是。

- **`fundeploy upgrade`：自升级收进二进制自己。**

  ```
  fundeploy upgrade                   # 升到最新版
  fundeploy upgrade --check           # 只看有没有新版本，不动手
  fundeploy upgrade --version 0.2.0   # 装指定版本，比当前旧也照做（这是回退的用法）
  ```

  不带 `--version` 时只升不降：远端不比本地新就直说「已经是最新的」然后退出，
  而不是把同一个版本再下一遍。要回退必须显式点名版本号。

  几处刻意的顺序：**先查能不能写，再联网**——十几兆下完才发现装在
  `/usr/local/bin` 下没权限，这个代价没必要付；**换上去之前先跑一次
  `--version`**——下坏了或者下成别的架构的话，换过去就是把人唯一能用来补救的
  工具也弄坏了，所以新的那份跑不起来就原样不动，并说清楚什么都没动过；
  **临时文件落在目标目录里**，最后那一步是同目录 rename（跨文件系统 rename 会
  失败，而 `~/.cache` 和 `/usr/local/bin` 经常不在一个盘上）。

  认得出自己是 `target/debug/fundeploy` 这种构建产物，这时直接拒绝——换掉它
  只会让下一次 `cargo build` 把它覆盖回去。目录不可写时（apt 装到 `/usr/bin`
  的那份就是）也拒绝，并说明该照原渠道升。

- **调 GitHub API 时认 `GITHUB_TOKEN` / `GH_TOKEN`。** 匿名限流是每小时 60 次、
  按出口 IP 算，一个办公室共用一个出口的话，装几个服务就撞上限——而症状是
  「查不到最新版本」，看不出跟限流有关。带上 token 是 5000 次。用这两个现成的
  变量名而不是自己起一个：`gh auth login` 过的机器和 GitHub Actions 已经在设
  它们。token 只发往 `api.github.com`，且判的是镜像改写**之后**的地址——配了
  `FUNDEPLOY_GITHUB_HUB_PROXY_PREFIX` 时，URL 会变成 `https://某代理/https://api.github.com/…`，
  照旧附上 Authorization 等于把凭据递给那个第三方代理。
- **CI 从 shellcheck + 冒烟脚本换成 Rust 那一套**：`cargo fmt --check`、
  `cargo clippy -D warnings`、Linux 和 macOS 上各跑一遍 `cargo test`，外加四个
  发布 target 的 release 构建。构建放进 CI 而不是只在 tag 上跑：musl 静态链接有
  它自己的失败方式（`ring` 要 C 编译器、交叉链接器找不到），只在打 tag 时才发现
  就意味着每修一次都要重打一个 tag。arm64 用原生 arm runner 构建，不交叉编译。
- 发布流程多一步 `verify-install`：Release 发出去之后，在 ubuntu x86_64 /
  ubuntu arm64 / macOS 上照 `install.sh` 真装一遍，核对版本号，再跑 `list` 和
  `upgrade --check`。装机脚本只有在有一个真实 release 时才跑得通，它坏了的代价
  又比别处都大——那是别人碰到这个项目的第一条命令。

### 修复

- `skills`（原 `skills-sync`）：装上去的仓库再也删不掉。同步的最后一步会把仓库
  里除 `.git` 之外的点目录删掉，而这些目录多半是被 git 跟踪着的
  （`.github/workflows` 几乎人手一份），于是从同步完成的那一刻起
  `git status --porcelain` 就一直挂着一片 deleted。`_is_dirty` 照它判，仓库永远
  是「有本地修改」，`_delete_one` 于是永远拒绝：`[跳过] 有本地修改，拒绝删除`。
  自带的 selftest 之所以过，是因为它在删除前先 `git reset --hard` 把那些目录又
  还了回来。现在「有改动」的判据是「除我们自己删的点目录之外还有别的变化」；
  认不出来的 porcelain 行（带引号转义的路径之类）一律算改动——判错的方向是拒绝
  删除，比误删安全。
- `doctor github`（原 `github-net`）：换通道从来没换干净过。`git config
  <键> <值>` 对多值键是**替换**而不是追加，而 `url.<base>.insteadOf` 正是多值
  键——Bash 版连写两条，第二条把第一条顶掉了。后果是 `fix_https` 实际没有改写
  `git@github.com:`（恰恰是它让人改用的那种写法），`fix_ssh22` 实际没有改写
  `https://github.com/`（恰恰是这个修复存在的理由）。另外每条 fix 只写自己的
  规则、不清别人的，于是从 SSH 换回 HTTPS 之后两套规则同时在，谁生效取决于 git
  匹配到哪条前缀更长。现在写规则一律用 `--add`，写之前先把 fundeploy 管的两个
  base 底下的规则全清掉。
- `doctor github --fix ssh22`（原 `fix_ssh22`）：不会再被上一次的 443 配置架空。
  Bash 版从不删 `~/.ssh/config` 里那个把 `github.com` 指向 `ssh.github.com:443`
  的区块，于是先 `fix_ssh443` 再 `fix_ssh22`，所有 ssh 流量还是走 443——改了等于
  没改，而两处配置看起来都是对的。
- `airflow`：端口配置从来没生效过。脚本设的是
  `AIRFLOW__WEBSERVER__WEB_SERVER_PORT`，而 Airflow 3 整个没有 `[webserver]` 这个
  配置段——API server 读的是 `[api]`。于是 Airflow 一直听在它自己的默认 8080 上，
  fundeploy 打印、探测、启动前查占用用的却都是 8806：Bash 版只是给了一行错地址
  （它起完进程就返回，不等端口），放到会等端口的 Rust 版里就是每次 `start` 必定
  超时。现在设 `AIRFLOW__API__PORT` / `AIRFLOW__API__HOST`。
- `open-pencil install-desktop`：改为从 release 的附件清单里按平台挑，不再照
  模板拼文件名。拼名字的做法一旦上游改了命名就是 404，而 release 里其实躺着一个
  能用的包；它还得为「取不到 latest」硬编一个兜底 tag（`v0.13.2`），于是网络一抖
  就去下一个一年前的版本。挑不到时会把这个 release 实际有哪些附件列出来，
  `OPEN_PENCIL_DESKTOP_ASSET` 仍可点名覆盖。顺带，上游没发 linux/arm64 包这件事
  现在是从附件清单得出的结论，而不是写死在脚本里的一句「不支持」。
- `brew install`：Bash 版跑官方安装器时多传了一个参数
  （`/bin/bash install.sh fundeploy`），而 `install.sh` 对认不出来的参数是
  `Unrecognized option: 'fundeploy'` 然后退出 1——也就是说这条路径根本装不上。
  那个 `fundeploy` 本来是 `bash -c "<脚本正文>" fundeploy` 里的 `$0`，改成下载到
  文件再执行之后，它就从「进程名」变成了「第一个参数」。现在不传。
- `cockpit-tools install`：同 `open-pencil install-desktop`，改为按附件清单挑，
  并且删掉了兜底 tag `COCKPIT_TOOLS_FALLBACK_TAG`（默认 `v1.0.4`）。那个兜底的
  实际效果是：GitHub API 一抖，安装器就安安静静地给你装一个 2024 年的版本——
  上游现在已经发到 1.3.68 了。取不到最新版就直接报错，不猜。
- 跑上游脚本（brew 的安装器/卸载器）用的临时目录从 `/tmp` 挪到用户缓存目录下。
  `/tmp` 是所有人都能写的，往里放一个名字可预测的脚本再执行它，等于把「谁先建出
  这个路径谁说了算」交给了别人。
- `dev python`（原 `tool python-env`）：`PYTHON_VERSION` 这个覆盖办法从来没生效
  过。脚本第 21 行无条件写了一句 `PYTHON_VERSION=""`，把调用者传进来的值直接
  抹掉，而下面 `cmd_pyenv_update` / `cmd_pyenv_uninstall` 取的是
  `${PYTHON_VERSION:-$DEFAULT_VERSION}`——于是无论设成什么，动的永远是 3.12。
  这两处的注释里还写着「可通过 PYTHON_VERSION 覆盖」。
- `dev python install`：不再谎报「环境已激活！」。Bash 版在自己的进程里
  `source` 了一遍 activate，然后拿 `$VIRTUAL_ENV = $ENV_PATH` 判断有没有激活
  成功——这个判断在脚本自己的进程里永远成立，所以它每次都报激活成功，而调用它
  的终端什么也没变。真正有用的那段提示（`source …/bin/activate` 怎么敲）写在
  `else` 分支里，永远执行不到。子进程改不了父 shell 的环境变量，现在只打印
  命令让人自己执行。
- `dev pnpm install --method source`：在新版 Node 上不再以一句
  `corepack: command not found` 收场。Bash 版只检查了 Node ≥ 16.17 就直接
  `corepack enable`，而 Node 25 起 corepack 不再随发行版打包。现在找不到
  corepack 就自动改走 npm 全局前缀，`FUNDEPLOY_PNPM_VIA_NPM=1` 可以强制走后者。
- `dev rust install --method source`：不再往 shell 配置里塞一行卸载后会报错的
  `source`。Bash 版跑 rustup 安装器时没加 `--no-modify-path`，rustup 便会往
  `~/.profile`、`~/.bashrc`、`~/.zshenv` 里追加 `. "$CARGO_HOME/env"`——指向的是
  `~/opt/rust/cargo/env`。等哪天把 `~/opt/rust` 删掉，每开一个新 shell 都报一次
  「No such file or directory」，而那行藏在 rc 文件末尾，很难联想到是装 Rust 时
  留下的。卸载时也先让 `rustup self uninstall` 自己收尾，再删目录。
- `dev nodejs install --method source`：版本号不再写死在脚本里。Bash 版默认装
  `22.14.0`，一个具体的小版本号写进脚本之后就不会再有人去动它，装到的只会越来
  越旧。现在问 `nodejs.org/dist/index.json` 取当前 LTS。
- `ai`（原 `ai-cli`）：卸载只清默认那条路。Bash 版不点名装法时装法就是
  `official`，于是 `fundeploy ai claude uninstall` 只去删官方安装器留下的那几个
  路径——用 npm 装的人跑它会得到一句成功，而 `claude` 还好端端在 PATH 上。现在
  不点名就四条路（official / brew / npm / pnpm）都走一遍，哪条本来就没装只记
  一句，不当错误。
- `ai`：装法写错时报的是装法不对，不是动作不对。Bash 版靠「第一个词是不是已知
  装法」来区分装法和动作，不是就当动作，于是 `fundeploy ai cursor npm install`
  报的是「未知动作: npm」——真正的问题是 cursor 只有 official 这一条路，照着那
  句话去查动作名永远查不出所以然。现在直接说「cursor 没有 npm 这条路，它支持
  的是: official」。
- `ai`：卸载只问一次。Bash 版对每个要删的路径各弹一次确认，卸一个 codex 要连
  按三次 y，而养成闭眼按 y 的习惯之后，确认这道关就形同虚设了。

## [0.1.18]

### 新增

- `paperclip`：补上上游早已有、而本脚本一直没暴露的一等公民命令：
  `check [canary|prod]`（只查版本不改动任何东西）、`rollback`（回退到 install store
  里保留的上一份 payload）、`backup`（官方 `db:backup`）、`doctor [--repair]`、
  `health`、`channels`，以及
  `service-install`（透传 `--enable-linger` / `--no-start-on-login` /
  `--no-start-now`——此前 fundeploy 管理的 Paperclip 根本不会开机自启：`start` 内部的
  `ensureCurrent()` 会自愈 systemd 单元文件，但 `systemctl --user enable` 和
  `loginctl enable-linger` 只有官方 `service install` 会做）。
- `paperclip`：新增 `PAPERCLIP_UPGRADE_YES=1`，用于在非交互环境确认降级。

- `funread-web`：新增 `upgrade`（装完额外打印前后端的版本对比）与
  `rollback <版本>`（前后端一起钉到同一版本，回退前在跑就装完自动起回来）。
  `rollback` 会先用 `npm view` 确认私有源上真有这个版本再动安装——`install` 的
  顺序是先后端后前端，前端拉不到而后端已经降级的话，会停在前后端版本不一致的
  中间态上。npm 上 `@farfarfun/funread-web` 的版本号是跳着的（`1.1.106`–`1.1.108`
  三次发版都因到 Codeup registry 的链路丢包而从未发布成功），所以这个前置检查
  不是多余的。

### 修复

- `paperclip`：`upgrade` 把官方 `update --check --json` 的退出码 **10** 当成了失败。
  这个退出码是「有可用更新」的信号（payload 里 `if (options.check) process.exitCode = 10`，
  性质同 `diff`），不是错误——于是唯一真正需要升级的那种情况反而被判成「拿不到版本
  信息」。现在只看 stdout 能不能解析成 JSON，不看返回值。顺带给 `paperclip_cli` 加了
  `PAPERCLIP_CLI_NO_DIAGNOSE`，免得这种「非零但成功」的调用白跑一次损坏探测。
- `paperclip`：canary 机器上 `upgrade prod` 会卡死。`--latest` 指向的 stable 往往比
  当前 canary 版本旧（实测本机 `2026.1008.0-canary.4` → `2026.1005.0`），官方 `update`
  把这种情况当显式降级、要求 `-y` 确认，而此前脚本既不检测也不传，非交互环境里就停在
  上游的交互提示上。现在先问 `update --check --json`：`downgrade:true` 时经
  `fundeploy_confirm_destructive` 确认后才补 `-y`，`updateAvailable:false` 时直接跳过
  （不再白停一次服务）。
- `paperclip`：无服务管理器时 fundeploy 托管的后台进程缺 `</dev/null`，会继承调用者的
  终端 stdin，在交互式 shell 里一读 stdin 就被 `SIGTTIN` 停住，表现为「起来了但没响应」。
- `paperclip`：消毒用的临时 npmrc 会泄漏到 `/tmp`。它此前在脚本顶层无条件创建并挂
  `trap … EXIT`，而 `run` / `logs` 这些路径以 `exec` 收尾，`exec` 换掉进程映像后 EXIT
  trap 不会触发。现在改成只在 `install` / `upgrade` / `plugin install` 期间按官方
  `install.sh` 的做法写一份仅含两行 registry 的最小 npmrc（同时也是 npm allow-scripts
  缺陷 npm/cli#9783 的官方同款解法，替掉了原来那段 grep 式 `~/.npmrc` 消毒），并在
  `exec` 之前显式清理。
- `fundeploy service status`：Paperclip 的端口兜底值 `3100` 是错的，改为 `8804`——
  config.json 的 `server.port` 和 `paperclipai health` 默认探的都是 8804。配置还没生成
  时这里会去探一个根本没人监听的端口，把「未安装」显示成「已停止」。

- `funread-web`：`install` / `start` / `run` 会在默认配置目录下按需补一份
  `config.toml`，不补的话新机器上 `start` 必然失败。funread 1.1.111 的两个 CLI
  共享同一处缺陷：`server start` 把它自己解析出来的**默认** config 路径原样以
  `--config` 传给 fork 出的子进程，而上游对「显式指定的 config 不存在」是硬错误
  （默认路径不存在本来是容许的）。于是 `fundeploy service funread-web install`
  成功之后紧跟的 `start` 退出码 1，日志里只有一行
  `error: config file not found: ~/.config/farfarfun/funread-api/config.toml`，
  看着像包装坏了；而同一份安装 `funread-api server run` 一切正常。上游既没有
  `init` 也没有 `config` 子命令能生成这个文件，所以由部署侧补上——性质同
  funflix-web 当初补的「首次部署两步」。生成的内容只有 `[server]` 下的 host /
  port（真正生效的仍是 `start` 显式传的 `--host`/`--port`，这份文件主要是让上游
  的存在性检查过关），**不写库地址**——库地址属于两端共用的配置，写在上一层的
  `~/.farfarfun/funread/config.toml`（见下一条）。显式设置了
  `FUNREAD_WEB_{BACKEND,FRONTEND}_CONFIG` 时不代建：用户点名了那个文件，替他凭空
  造一份只会把「路径写错了」盖掉。

- `funread-web`：install/start/run 另外生成一份共享根配置
  `~/.farfarfun/funread/config.toml`（`_ensure_shared_config`）。funread 1.1.116 起
  库地址、管理端口令这些只认配置文件，`FUNREAD_DATABASE_URL` 等环境变量整个不读了，
  所以必须有个地方摆给操作者填。生成的内容**整份是注释**：填一个 `[database] url`
  进去就是决定这台机器连哪个库，脚本没有资格替操作者定；全注释的文件行为上等于
  没有这个文件（仍按 funsecret > 本地 SQLite 解析）。已存在则一个字都不动——那份
  文件里放的正是生产库地址和口令，升级时覆盖它等于静默把线上指到别的库上。
  权限 `0600`，两份服务配置也一并从 `0644` 收到 `0600`。

- `funread-web`：上面那份 `config.toml` 写到哪个目录，改成问 CLI（`server status`
  的输出里带 `config <路径>`，服务没在跑也会打），不再按 funread 的布局自己拼。
  funread 1.1.114 把配置目录从 `${XDG_CONFIG_HOME:-~/.config}/farfarfun/funread-{api,web}/`
  换到了 `~/.farfarfun/funread/{api,web}/`，而同一个 fundeploy 要同时伺候两种版本的
  机器——光是 `rollback 1.1.113` 就能把一台机器退回旧布局。两边任选一个写死，都会让
  这份配置建到 CLI 根本不看的位置，症状和它要修的那个缺陷一模一样（`start` 报
  `config file not found`），而且更难查：日志里明明有一行「已生成配置」。CLI 问不出来
  （没装、或上游改了输出格式）才退到新布局的硬编码值。

- `funread-web`：`upgrade` 改为「先停服 → 装新版 → 打印版本对比 → 原先在跑就起回来」，
  与 `rollback` 一致。此前它在跑着的进程底下直接换包：前端是全局 npm 包，node 还开着
  那棵树里的文件，装一半的产物会被新请求读到；后端换 uv tool / pip 环境同理。
  1.1.113 → 1.1.114 这一跳还多一个一次性的坑——旧进程的 PID 文件写在**旧**配置目录里，
  升级后的 CLI 只看新目录，于是 `stop` 报「not running」而端口还被占着，接着 `start`
  又撞 address in use。存活判断走端口探测而不是 PID 文件，正是因为 PID 文件的位置本身
  会随版本变，端口不会。

- `funread-web`：「先停服」带来的新失败模式——停完之后换包挂掉，`set -e` 让脚本就地
  退出，用户只看到包管理器那几行报错，没有一句说机器上的服务现在是停的。实测撞到过：
  私有 npm 源一个 `ETIMEDOUT`，结果后端已经换成新版、前端还是旧版、两个端口都没人听，
  而输出里一个字都没提服务状态。`upgrade` / `rollback` 现在在停服后挂一个 EXIT 钩子，
  换包没走完就退出时明确提示「现在是停着的」并给出恢复办法；换包成功后立刻撤掉，
  免得后面 `start` 自己的报错上再附一句无关的。

- `fundeploy service status`：`FUNREAD_WEB_SERVICE_HOME` 在「安装路径」那行仍被
  展开，但它的赋值已随 funread-web 改走正式包而删掉了，`set -u` 下该行会以
  `unbound variable` 中断整条命令：状态表已经打出来了，但「安装路径」及其后的
  「详情」「工具」三行全丢，退出码变成 1——拿退出码判断的调用方会把一次正常的
  status 当成失败。funread-web 不再有安装目录（后端在 uv tool / pip 的环境里，
  前端是全局 npm 包），已从该行移除，与同样装正式包的 funflix-web /
  funlesson-web 一致。

- `celery`：Flower 的 basic auth 凭据不再以 `--basic-auth=user:pass` 形式进入
  命令行。守护进程存活期间该参数一直在 `/proc/<pid>/cmdline` 里，而这个文件对
  本机所有用户可读，`ps aux | grep flower` 即可读到明文（SPEC §9.1）。改为
  `export FLOWER_BASIC_AUTH`，由 Flower 自己的 `FLOWER_*` 环境变量机制读取。
- `celery`：`run-flower`（前台）此前既不传认证也不告警，用户设了
  `FLOWER_BASIC_AUTH` 仍会跑出一个完全无认证的 Flower。认证与「对外监听且无
  认证」的告警统一下沉到 `prepare_flower_env()`，`start-flower` 与 `run-flower`
  行为一致。
- `sub2api`：`sub2api.env` 里用户自己写的 `SERVER_HOST` / `SERVER_PORT` 不再被
  脚本的代码默认值覆盖。按 SPEC §9.3 的「环境变量 > 配置文件 > 代码内默认值」，
  只有显式设置了 `SUB2API_HOST` / `SUB2API_PORT` 时才覆盖配置文件的值。
- `fundeploy service status`：Celery 行里残留的 `FLOWER_ADDRESS` 默认值
  `0.0.0.0` 与 `services/celery/setup.sh` 的实际默认值 `127.0.0.1` 不一致，
  会展示出一个服务其实不会绑定的监听地址，已对齐。

### 变更

- **破坏性**：`paperclip` 整体重构为「直驱官方 CLI」，删掉约 430 行自造机制
  （1240 行 → 1074 行，其中代码 742 行，与兄弟脚本的 589 / 614 持平）。
  删掉的两大块：
  （1）约 295 行 npm 镜像预装机制。它要解决「依赖树 1.4G 下得慢」，但上游把
  `registry.npmjs.org` 在 `install.sh` 和 `cli/src/commands/install.ts` 两层硬钉死
  （`--registry=`、`--@paperclipai:registry=`、四种 `NPM_CONFIG_*` 拼写、一次性
  `npm_config_userconfig`），这是有上游测试守着的防 registry 投毒设计，任何 flag 和
  环境变量都换不掉镜像——这套机制在解决一个它解决不了的问题，而且自己带来了 bug。
  （2）约 130 行 macOS arm64 embedded-postgres 补丁（pnpm `allowBuilds`、
  `hydrate-symlinks`、删 launchd 里的 `DATABASE_URL`），改为调用上游现成的
  `doctor --repair --yes`（它会检查配置、install store、manifest、`current` 链接、
  shim、PATH、目录权限等并尝试修复）；刻意放在 `start` 之前，因为官方
  `paperclipai run` 自己也是 `onboard` + `doctor --repair` 之后才启动。
  手搓的版本比较与「already up to date」判断也交回 `update --check --json`。
  **迁移**：`PAPERCLIP_NPM_MIRROR` 与 `PAPERCLIP_NPM_PREFETCH` 两个变量已移除
  （设了也从来没能改变实际下载源）。受限网络改用 `PAPERCLIP_HTTPS_PROXY` +
  `PAPERCLIP_NO_PROXY`——那是唯一能透进官方 managed install 的通道；
  `PAPERCLIP_NPM_FETCH_*`（取回重试与超时）和 `PAPERCLIP_INSTALL_VIA_NPX` 保留。
- **破坏性**：`paperclip` 的 `upgrade` / `rollback` 不再自动备份数据库，备份提成独立的
  `backup` 命令（转调官方 `db:backup`）。`update` 换的是 payload，不会动
  `instances/<id>/db` 里的数据，payload 要退回去有 `rollback`；而一次备份在实测机器上
  是 370M、耗时数十秒，每次升级都付这个代价并不划算。给官方 `update` 一并传了
  `--no-backup`，把它自己那次 pre-update 备份也关掉。顺带去掉了「服务停着时升级会在第
  一步中止」这个失败模式——embedded PostgreSQL 是服务的子进程、随服务一起停，停着时
  `db:backup` 必然连不上，`set -e` 下整条升级就断在那里；现在没有这一步了。
  **迁移**：原先依赖升级自动留一份备份的，改为先 `backup` 再 `upgrade`。embedded
  PostgreSQL 模式下 `backup` 要求服务在跑，停着时会在调用上游之前就报错并提示先
  `start`，而不是丢一个连接错误出来；用外部数据库的实例与服务生命周期无关，停着也能备。
- `paperclip`：仍不采用官方文档推荐的 `curl -O https://paperclip.ing/install.sh`
  + sha256 引导路径，因为那条路径当前是坏的：`paperclip.ing` 上那份（文档指向、
  也是唯一发布了 `.sha256` 的）落后于 master 的 #11792，给 CLI 传了 `install` 并不
  接受的 `--no-prompt`（交接直接失败）、`MIN_NODE_MAJOR=20`（上游已要求 24.11+）、
  用 `${value,,}` 这种 bash 4 语法在 macOS 自带 bash 3.2 上崩、第三方脚本无 SHA 固定
  （上游 issue #10739 在跟踪）；master 上那份正确的 `scripts/install.sh` 没有发布
  checksum（`.sha256` 返回 404），无从校验；它的 Node 引导还会在见到 `~/.nvm` 时
  直接 `fail`。而只要 PATH 上已有 node 24+，install.sh 的 Node 引导块会被整个跳过，
  它就只剩 `npx --yes --registry=… paperclipai@latest install --yes` 这一行——也正是
  本脚本执行的那条。终态与 install.sh 一致，但不引入一个过期、且在装了 nvm 的机器上
  根本跑不起来的脚本依赖。
- `paperclip`：`nvm use 24` 不再在每次 CLI 调用前执行。managed install 的 shim
  已经用绝对路径把自己那份 node 钉死（`exec '<nvm>/…/v24.x/bin/node' '<payload>/dist/index.js'`），
  经 shim 的调用自带正确的 node；版本门槛因此收窄到真正需要它的 npx 引导路径
  （那时还没有 shim）与 `run` / `start`，`status` / `logs` / `stop` 这些热路径不再
  每次 `source nvm.sh` 并改写 PATH。
- `paperclip`：交互菜单的选项补成「命令 + 中文描述」并用 `pick="${pick%% *}"` 还原，
  与仓库其余服务一致（此前是裸命令名，是全仓唯一的偏离）。
- **破坏性**：`sub2api` 默认监听地址由 `0.0.0.0` 改为 `127.0.0.1`。sub2api 是
  订阅 API 网关，`data/` 下存着上游渠道 key 与用户 token，默认绑全网卡不符合
  SPEC §9.3 的安全默认值要求；同仓库其余服务默认都是 `127.0.0.1`，
  `fundeploy service status` 的探活逻辑也一直按 `127.0.0.1` 写。
  **迁移**：需要对外暴露时显式设置 `SUB2API_HOST=0.0.0.0`（或在
  `~/opt/sub2api/data/sub2api.env` 里写 `SERVER_HOST=0.0.0.0`），`start` / `run`
  会提示确认防火墙与访问控制。
- `celery`：虚拟环境与依赖改为优先走 `uv`（`uv venv` + `uv pip install`，与同
  仓库 airflow 服务一致），缺 `uv` 时退回 `python3 -m venv` + `pip`；依赖不再是
  裸包名，改为带版本下限的 `celery>=5.3` / `redis>=5.0` / `flower>=2.0`，可用
  `CELERY_PKG_SPEC` / `CELERY_REDIS_PKG_SPEC` / `CELERY_FLOWER_PKG_SPEC` 覆盖
  （SPEC §5）。
- README：底部组织介绍区块补上 SPEC §12.2 要求的 `---` 分隔线，并去掉与区块末行
  重复的独立「许可证」小节；服务清单补上 `funmill` / `funfluid-web` /
  `fungame-web`，并写明「默认只监听回环」的约定。
- CHANGELOG：修正 `[0.1.15]` 的错误归属（见下）。
- **破坏性**：`funread-web` 从「源码构建」改为「装正式包 + 转发上游 CLI」，与
  funflix-web / funlesson-web 对齐。后端由 `funread[api]` 装进脚本自建的 venv
  改为装 PyPI 上的 `funread-api`（uv tool / pip 自适应，复用机器上已有的那一份）；
  前端由 `git clone` + `pnpm run build` 改为装私有 npm 上的
  `@farfarfun/funread-web`。PID 与日志不再由本脚本 `nohup` 托管，改由两个 CLI
  各自写在实际解析到的 config 同目录下，`start`/`run`/`stop`/`status` 只是转发
  `funread-api server …` 与 `funread-web server …`。
  **迁移**：`FUNREAD_WEB_SERVICE_HOME` / `FUNREAD_WEB_GIT_URL` /
  `FUNREAD_WEB_GIT_REF` / `FUNREAD_WEB_PYTHON_BIN` 四个变量已移除，改用
  `FUNREAD_WEB_{BACKEND,FRONTEND}_{PACKAGE,VERSION,CONFIG}`；旧的
  `~/opt/funread-web` 不会被自动清理，确认服务已按新路径起得来之后手动删掉即可。
  前端在私有源上，首次安装前机器上需要一次性的 scope 映射
  （`npm config set @farfarfun:registry …`，pnpm 不接受命令行覆盖）。
  `install`/`upgrade`/`rollback` 故意不转调上游 CLI 的同名命令：`funread-api upgrade`
  写死 `uv tool install`、`funread-web upgrade` 写死 `npm install -g`，在用 pip /
  pnpm 装过的机器上会各装出第二份副本，PATH 里仍是旧版——升级看着成功其实没生效。
  `install` 后会核对「刚装的那份是不是 PATH 上会跑到的那份」，不一致就告警。
- `funflix-web`：前端 npm 包名跟随上游改为 `@farfarfun/funflix-web`（bin 名仍是
  `funflix-web`，与 `funfluid-web` 的写法对齐）。`install` / `upgrade` 会先摘掉
  改 scope 前装的裸名全局包——它占着 `bin/funflix-web`，不清掉新包会在建 bin
  软链时直接 `EEXIST` 失败。显式把 `FUNFLIX_WEB_FRONTEND_PACKAGE` 指回裸名时
  不做这个清理。

### 废弃

- `paperclip`：`PAPERCLIP_NPM_MIRROR` / `PAPERCLIP_NPM_PREFETCH` 已移除（见「变更」
  里的迁移说明）。`update` 作为 `upgrade` 的别名继续保留。

## [0.1.17] - 2026-09-27

### 新增

- 无

### 修复

- `_pip_install_pkg`：修正 `uv pip install` / `python3 -m pip install` 的参数
  拼接，此前的写法在部分路径下会把包名与选项拼错。
- `fungame-web` / `funlesson-web` / `funflix-web`：`uv pip install` 补回 `-U`，
  否则已安装过的包不会被升级到目标版本。

### 变更

- 无

### 废弃

- 无

## [0.1.16] - 未发布

> 没有 `v0.1.16` tag，也没有对应 GitHub Release；`VERSION` 曾短暂停在
> `0.1.16`，这些改动实际随 `0.1.17` 一起发布。

### 新增

- 新增 `funfluid-web` 服务模块（install / start / stop / restart / status）。
- 新增 `fungame-web` 服务模块（后端 + 管理端 + 客户端三进程编排）。
- 新增 `funlesson-web` 服务模块（前后端一体化管理）。
- 新增 `sync-gitee` workflow，每次 push 镜像到 Gitee。
- README 补充「关于 farfarfun」组织介绍区块，末尾附 MIT 协议说明句。

### 修复

- `paperclip`：改为从 npm userconfig 里清掉 `allow-scripts`，只清环境变量不够。
- 前端类服务：修正 pnpm 的 `@latest` dist-tag 解析问题。
- `funfluid-web`：显式导出 `FUNFLUID_CONFIG_DIR` / `FUNFLUID_DATA_DIR`。
- `funflix-web`：后端改用 `funflix-api`（跟随 funflix 仓库拆分）。
- `sync-gitee`：只推 heads/tags，GitHub 的 PR ref 会被 Gitee 当隐藏 ref 拒绝。
- `install_smoke`：收紧 PATH 时保持 node 可达，否则服务 status 用例假失败。
- `.gitignore` 补齐 `__pycache__/`、`*.pyc`、`*.db`、`*.rar`、`.venv/`、`.run/`、
  `node_modules/` 等规范要求的规则。

### 变更

- 各服务模块的 `update` 子命令统一改名为 `upgrade`。
- 前端包安装优先用 pnpm，没有 pnpm 时退回 npm。
- `paperclip` 改为复用上游自带的服务管理命令。

### 废弃

- 服务模块的 `update` 子命令（由 `upgrade` 取代）。

## [0.1.15] - 2026-08-11

### 变更

- 无（见下方说明）

> 更正：原先记在本版本下的「README 组织介绍区块」与「`.gitignore` 补齐」两条
> 实际是 2026-09-03 的提交，晚于 `v0.1.15`（2026-08-11）—— `v0.1.15` 这个 tag 的
> 树里既没有 `CHANGELOG.md`，README 也还是旧的「许可证 / [MIT License](LICENSE)」
> 结尾。两条已移到实际落地的 `[0.1.16]` 下。

---

> 关于 SPEC §6「服务脚本」：`scripts/setup.sh` 统一入口、`start`/`run` 强制带
> `dev`/`prod`、运行时文件迁到仓库内 `.run/` 这三项，在本仓库不适用，已在
> todo-list#383 与 #665 中说明理由并留待仓库所有者确认：
>
> - fundeploy 自身不含任何长期运行服务。`scripts/services/*/setup.sh` 管的是
>   **安装在用户本机 `~/opt/<service>/` 下的第三方 / 其他仓库的服务**，
>   fundeploy 只是部署器。
> - fundeploy 以 apt / Homebrew / `~/.local/fundeploy` 形式分发，运行时根本不存在
>   「仓库工作区」，把被管服务的 PID/日志写到仓库内 `.run/` 既无处可写也会丢失。
> - 被管服务没有天然的 dev/prod 划分：主机上只有一份正式安装（PyPI / npm /
>   GitHub Release 的发布产物），不存在「从源码起」这条路径。
> - 统一入口已经有了，是 `fundeploy` 命令本身（`scripts/fundeploy.sh`，
>   `域 → 模块 → 动作`），而不是 `scripts/setup.sh`。
