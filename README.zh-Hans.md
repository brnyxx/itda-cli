<p align="center"><img src="assets/hero.png" alt="itda：一只手沿着一根线把一张小小的网站卡片递给 AI 代理。在 itda 后面输入一个地址，你的 AI 就能使用这个站点。" width="100%"></p>

<p align="center"><strong>在 itda 后面输入一个地址，你的 AI 就能使用这个站点。</strong></p>

<p align="center"><a href="README.md">English</a> | <a href="README.ko.md">한국어</a> | <a href="README.ja.md">日本語</a> | <b>简体中文</b> | <a href="README.zh-Hant.md">繁體中文</a> | <a href="README.es.md">Español</a> | <a href="README.fr.md">Français</a> | <a href="README.de.md">Deutsch</a> | <a href="README.pt-BR.md">Português (Brasil)</a> | <a href="README.vi.md">Tiếng Việt</a> | <a href="README.id.md">Bahasa Indonesia</a> | <a href="README.ru.md">Русский</a></p>

<p align="center">
  <a href="https://github.com/brnyxx/itda-cli/actions/workflows/ci.yml"><img src="https://github.com/brnyxx/itda-cli/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/brnyxx/itda-cli/releases"><img src="https://img.shields.io/github/v/release/brnyxx/itda-cli?include_prereleases&amp;label=release" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue" alt="License: Apache-2.0"></a>
  <a href="https://brnyxx.github.io/itda-cli/docs/">文档</a>
</p>

给 itda 一个地址。它弄清这个站点能做什么，并实际试一试。它交给你的
AI 代理一份简短的、用大白话写的操作清单。读取直接就能用。在你说“可以”
之前，站点上什么都不会改。它在你自己的电脑上运行，对需要登录的老式
内网同样有效。

它面向你公司运营的站点和你有账户的服务，而不是整个互联网。

<p align="center"><img src="assets/setup.gif" alt="第一次运行：itda 询问地址，粘贴 news.ycombinator.com 之后，它找到一个 30 条记录的列表、一个表单和一个订阅源，把两个读取都试了一次，保存站点，再按一次 Enter 就连接了 Claude Desktop" width="860"></p>

<details>
<summary>同一次运行的文字版</summary>

```console
$ itda news.ycombinator.com
1/5  正在查看 news.ycombinator.com...
    这个页面不用登录就能打开。
    找到：一个有 30 条记录的列表, 一个表单, 一个订阅源。
2/5  代理将能做的事：
    读取   home         List what Hacker News shows  (输入：p)
    读取   feed         Read the RSS feed
3/5  正在逐一测试每个读取操作...
    ✓ home         30 条记录
    ✓ feed         成功
4/5  已保存为 'news-ycombinator'。
    连接 Claude Desktop？[Y/n]
5/5  正在把 itda 添加到 Claude Desktop...
已把 itda 添加到 Claude Desktop（~/Library/Application Support/Claude/claude_desktop_config.json）。
打开 Claude Desktop，itda 就在那里。

完成。向你的代理问点 'news-ycombinator' 上的事，比如 "用 news-ycombinator List what Hacker News shows"。
```

</details>

对你的代理说：*“用 news-ycombinator 列出 Hacker News 显示的内容”*。需要
登录表单的站点会改为向你要用户名和密码。itda 替你填写站点自己的
表单，并把会话留在你的电脑上。如果这样还不够（单点登录、一次性验证码），
它会打开你自己的浏览器。你照常登录和使用站点，itda 在旁边观察。

<p align="center"><img src="assets/flow.svg" alt="在浏览器里录制一次网站；itda 据此起草命令。代理调用它们；更改要等一个人批准。" width="860"></p>

> 状态：早期，但已用于真实站点。运行时、CLI、MCP 服务器、批准（在终端里或
> 在代理应用内）、审计日志、从 OpenAPI 文档、浏览器录制或抓取的页面起草、
> API 密钥、浏览器登录和 CSRF 预检步骤今天都已可用。下一步是签名的站点包。
> 见 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)。

## 为什么

操控浏览器的代理每一页都重新读，每张截图或无障碍快照都要花掉数千个
token。页面叫它点哪儿它就点哪儿，布局一变就悄悄坏掉。itda 把这些工作
挪到了安装那一步：

| | 操控浏览器的代理 | itda |
|---|---|---|
| 代理看到的 | 整页，每一步 | 一个操作的带类型结果，上限 24 KB |
| 改版或已登出的页面 | 当作数据读进去 | 根据声明的响应检测出来（`response_mismatch`、`auth_required`） |
| 站点上的更改 | 模型说了算 | 针对精确输入的一次性批准，由人在终端或代理应用里给出 |
| 页面上写着“忽略之前的指令” | 进入提示词 | 标记为不可信数据，永远不是工具 |
| 凭据 | 由模型输入到页面里 | 在操作系统密钥环中；代理只看到账户 id |
| 被屏蔽 | 重试、换 IP、冒充浏览器 | 在你自己装好的浏览器里把读取再试一次，记住哪种方式行得通，什么都不暂停 |
| 发生了什么 | 聊天记录 | 可以验证的哈希链审计日志 |

## 能做什么，不能做什么

这里的每一行都是今天的代码真正做到的。`itda setup` 会说明它遇到的是哪
种情况；`itda draft page` 或 `itda record` 会说明它们漏掉了什么。

**现在就能用**

- 服务器绘制的页面：表格、列表和重复的卡片变成记录，只有一件事物的页面
  变成对其字段的读取。搜索表单变成读取，发布表单变成更改，订阅源一并
  带上。
- 为脚本内嵌数据的页面（`__NEXT_DATA__`、`ld+json`、Apollo 或预加载
  状态）：其中每个列表一个读取。
- 由脚本绘制、调用返回 JSON 的 API 的页面，比如 React 或 Vue 门户：
  `itda setup` 在页面上打开你的浏览器，观察它自己加载了什么，并从这些
  调用起草。它不点击也不输入。
- 需要 CSRF 令牌的表单：预检步骤在每次批准的提交之前从表单页面读取一个
  新的令牌。
- 作为框架外壳的页面：内容放在同一站点的 `<iframe>` 里，往往有自己的
  登录。itda 把每个框架当作站点的一个页面来读取，框架要求时先登录。
- Cookie 会话：通过站点自己的表单用你的用户名和密码登录，或者用你自己
  的浏览器处理单点登录和一次性验证码。地址中携带的会话（`?sid=...`）
  同样可以。
- 站点发布的 OpenAPI 或 Swagger 说明，以及回应内省的 GraphQL 端点：每个
  操作或查询字段一个读取。
- 运行 GitLab、Jira、Confluence、Redmine、Discourse、MediaWiki、
  WordPress、Gitea 或 Forgejo 的站点：使用其自身的 API，来自现成模板。
- 文件的接收与发送：页面标为附件的链接会变成一个读取，把文件保存到你的电脑并返回路径、大小和哈希，从不返回字节。带文件控件的表单会变成一个发送你指定文件的更改；你按名称、大小和哈希批准它。
- 任何你自己录制过使用过程的东西，在浏览器里或作为 HAR 文件。
- 挑战页面后面的页面，只要你电脑上的浏览器能过。itda 在那个浏览器里把读取
  再试一次，窗口隐藏，只给这一个站点和账户用，并记住哪种方式行得通，下次调用
  就从那里开始。浏览器在 CLI 调用之间保持运行，闲置五分钟后关闭；换个搜索词
  不会重启它。页面加载的一切都经过和直接请求一样的地址和读取检查。见
  [传输策略](docs/HONEST_CLIENT.md)。

**录制后能用**

- 直接抓取时什么都不显示、自己也不加载任何东西的页面。它只在你点击、
  搜索或登录之后才去获取。`itda record` 观察你使用它，并从看到的调用
  起草。

**不能用**

- WebSocket。itda 只说普通 HTTP，录制也不保留 socket 流量。
- 对每个请求签名的 API（每次调用都计算签名或 nonce）。itda 在密钥环里
  保存固定的令牌或 API 密钥。它不会复现每次调用都变化的值，所以站点会
  拒绝请求。
- 由 HTML 片段拼装的页面（htmx 之类）。这不是有意为之：itda 仍然可能
  读到一个包含整张表格的片段，但读不到脚本拿它做了什么。
- 画在画布或图片里的内容。itda 读取 HTML、JSON 和 XML 中的文字；它不看
  像素。
- 你的浏览器也过不去的挑战，或者验证码。itda 不解验证码。这次读取失败并如实
  报告；什么都不暂停。见 [docs/HONEST_CLIENT.md](docs/HONEST_CLIENT.md)。
- 任何你的账户看不到的东西。itda 没有自己的账户；它用你的。

## 站点变了之后

`itda check <site>` 运行每一个不需要输入的读取，从不发送更改。它以一个
结论收尾：`ok`、`signed_out`、`site_changed`、`blocked` 或 `unreachable`。
退出码随之而定（`site_changed` 为 11），所以定时任务可以在代理撞上之前
提醒你。在 MCP 上，`itda_check` 返回同样的内容。
`itda fsck <site>` 读取 itda 记下的本地尝试记录（只有结构和判定，没有值），列出从未尝试的、需要输入的以及时而失败的操作，并附上要运行的命令。

当结论是 `site_changed` 时，`itda refresh <site>` 从当初设置时的来源重新
起草站点：它的页面、API 说明、GraphQL 端点，或者保留下来的录制形状，每个
值都已清空。它逐个操作显示会改什么，在你说“可以”之前什么都不写。itda
保留你手工编辑过的操作，并更新新草稿改动了的操作。来源不再起草出的操作，
只有在其读取也失败时才会被删掉；每个新操作先试一次，有应答才加入。itda
撤回等待该站点的批准，并在之后立刻再检查一遍站点。用 `itda add` 安装的
定义没有来源，不能刷新；请手工编辑。

这就是取舍。浏览器代理每次运行都在适应，因此慢，而且不确定。itda 是一个
请求、每次同样的答案；遇到挑战就在你的浏览器里再试一次，答案仍然一样。站点
一变，它就大声报错。`docs/TRANSPORT_BENCHMARKS.md` 里有实测数字，包括 itda
还没达到的那一项。

## 与浏览器代理并用

Playwright MCP、browser-use 和计算机使用在每个任务上都操控浏览器。itda
不取代它们：只用一次的页面，或者上面列表里 itda 不覆盖的站点，归它们。
itda 面向代理会反复调用的站点。在浏览器里录制一次，审阅操作，然后通过
HTTP 运行它们，得到带类型的回答、每个更改的一次性批准和一份日志。两个
都用：用浏览器代理探索，用 itda 跑每天都要跑的调用。

## 快速开始

```sh
# macOS 和 Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.sh | sh
```

```powershell
# Windows
powershell -ExecutionPolicy Bypass -c "irm https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.ps1 | iex"
```

从 [Releases](https://github.com/brnyxx/itda-cli/releases) 下载压缩包，
或者用 `cargo install --git https://github.com/brnyxx/itda-cli itda-cli`
从源码构建。

Claude Desktop，不用终端：下载
[macOS（Apple 芯片）](https://github.com/brnyxx/itda-cli/releases/latest/download/itda-aarch64-apple-darwin.mcpb)、
[macOS（Intel）](https://github.com/brnyxx/itda-cli/releases/latest/download/itda-x86_64-apple-darwin.mcpb) 或
[Windows](https://github.com/brnyxx/itda-cli/releases/latest/download/itda-x86_64-pc-windows-msvc.mcpb) 的包并打开文件。Claude
Desktop 会安装 itda 及其工具，“use the-site-you-use.example” 就能在对话里
设置一个站点。批准更改仍然在终端里用 `itda approve` 完成，所以要做更改
的话也请安装命令行工具。

输入 `itda` 和一个你使用的站点地址。它找到你电脑上的代理应用，按一次
Enter 就连接好。它会说 12 种语言：由 `LANG` 选择，`ITDA_LANG` 可以覆盖。只输入 `itda` 会询问
地址；`itda setup <address>` 接受供脚本使用的参数。

itda 能认出运行已知软件的站点。当页面说它是 GitLab、Jira、Confluence、
Redmine、Discourse、MediaWiki、WordPress 或 Gitea（也包括 Forgejo），并且
向该软件自身 API 的一次请求确认了这一点，代理就从现成的模板
（[`sites/software/`](sites/software/)）得到该软件真正的 API，并按该软件
的方式登录。页面本身显示的内容也会一并加入。`itda setup --as jira` 强制
使用某个模板；`--no-template` 像普通页面一样读取。

由脚本绘制的页面只给 itda 看一个标题，所以 itda 会在它上面打开你的浏览器
几秒钟。它观察页面自己加载了什么，并从这些调用起草。不点击也不输入，窗口
会自行关闭。只有仍然什么都不显示、或者需要登录的页面，才会请你在浏览器
里使用站点，由 itda 观察。

连接好代理之后，你也可以直接对它说：“帮我设置 news.ycombinator.com”。它
调用 `itda_setup`。itda 以同样的方式查看站点、起草并试一试。站点的操作
会作为工具出现在同一个对话里。脚本页面会在你屏幕上的浏览器窗口里短暂
打开。当某个读取说需要登录时，`itda_login` 打开你自己的浏览器；代理只
看到账户 id。

跳过终端，把下面这段连同你使用的站点地址交给 Claude Code、Cursor 或
Codex：

```text
请为我安装 itda 并设置 the-site-you-use.example。
1. 安装：curl -LsSf https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.sh | sh
   （Windows 上：irm https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.ps1 | iex）
2. 运行：itda setup https://the-site-you-use.example --client claude-code -y
   （如果你是 cursor、codex 或 claude-desktop，就用相应的名字）
3. 先阅读 https://github.com/brnyxx/itda-cli/blob/main/AGENTS.md 这个文件，然后通过 itda 使用这个站点。
   读取直接做就行。任何会更改站点的事都要等我批准。
   如果某个读取说需要登录，告诉我运行：itda login <site> --browser
```

这些环节各自都是独立的命令。对于不用登录就能打开的页面，把它们的地址给
itda。表格、列表和重复的卡片变成记录。只有一件事物的页面（一个包、一篇
文章、一件商品）变成对其标题、正文和带标签的值的读取。链接共享同一形状
的列表会得到一个按 id 读取单条记录的操作。搜索表单变成读取；发布表单变成
更改。订阅源一并带上。页面提到的 GraphQL 端点会被问一次它提供什么，每个
查询字段一个读取。当站点发布了 API 说明时，它的读取也加入草稿：

```sh
itda draft page https://example.com/board --install   # 显示它在页面上找到了什么
itda call example.get-board --page 2 --json
itda check example                                     # 页面还和定义一致吗？
```

对于需要登录表单的站点，把页面和你的用户名交给 `itda setup`。它把站点自己的表单发送一次，用的是你输入的密码。它把会话保存在系统
密钥环里，并按你登录后看到的页面起草。定义记住了这个表单，所以会话过期
时 `itda login board` 会重新登录：

```sh
itda setup https://intranet.example/board --name board --user me   # 询问密码
itda call board.list-notices --json                 # 读取立刻运行
itda call board.create-notice --title "Server maintenance"   # 更改等待你的批准
itda login board --user me                          # 会话过期时
```

对于单点登录或一次性验证码，改为录制你自己使用站点的过程。itda 在一个
新窗口里打开你自己的浏览器。它观察你登录、搜索和提交表单时页面向服务器
要了什么；它从不点击也不输入：

```sh
itda record https://intranet.example/board --name board --install   # 完成后按 Enter
                                                    # 然后对保留它用过的会话或令牌回答“是”
itda mcp install claude-desktop                     # 或者让你的代理调用它们
```

`examples/lab` 运行 Gitea、Redmine、WordPress、MediaWiki、一个小型旧式内网，
以及三个更难看的案例，各自都在登录之后：一个 iframe 里的公告栏、一张脚本
从 XHR 绘制的表格、一个带在 URL 里的会话。一套测试把每个案例从设置一直走
到一次批准的更改。

录制留在内存里，到达草稿的只有每次调用的形状：没有头部、cookie 或你输入
的值。从浏览器开发者工具保存的 HAR 文件用 `itda draft har` 的效果相同。

公开 API 不需要录制。[sites/](sites/README.md) 里这些现成的站点内置在
程序里，不需要账户：

<p><a href="sites/github.yaml"><img src="assets/brands/github.svg" width="36" height="36" alt="GitHub"></a> <a href="sites/npm.yaml"><img src="assets/brands/npm.svg" width="36" height="36" alt="npm"></a> <a href="sites/crates-io.yaml"><img src="assets/brands/crates-io.svg" width="36" height="36" alt="crates.io"></a> <a href="sites/pypi.yaml"><img src="assets/brands/pypi.svg" width="36" height="36" alt="PyPI"></a></p>

```sh
itda add github      # 或 npm、crates-io、pypi
itda call github.repo --owner rust-lang --repo rust
```

试试内置的沙盒，一个只监听 127.0.0.1 的假内网：按
[examples/sandbox](examples/sandbox/README.md) 操作。一个站点定义就是一个
YAML 文件。`itda add spec.yaml` 先显示它把请求发到哪里、哪些操作会更改
站点，然后安装它。`itda doctor` 打印数据目录的位置。
[docs/SPEC.md](docs/SPEC.md) 说明如何编写定义。

```sh
itda setup <url>                         # 页面或录制 -> 命名的操作 -> 试过 -> 代理，带提问
itda draft page <url>... [--install]     # 从你使用的页面起草一个站点
itda record <url> [--install]            # 或者从你在浏览器里的使用过程起草，在登录之后
itda draft har <file> / openapi <file>   # 或者从保存的录制或 API 说明起草
itda add spec.yaml / remove <site>       # 安装或移除一个站点（也可以是 URL 或 github）
itda ls                                  # 已安装的站点及其操作
itda describe <site>                     # 它们的输入，以及每个可复制的命令
itda call <site>.<action> --input value  # 运行一个；--json 供代理使用
itda login <site> --user me              # 通过站点自己的表单登录（密码来自 stdin 或提问）
echo "$KEY" | itda login <site> --secret api-key   # 或者把 API 密钥存进操作系统密钥环
itda login <site> --browser              # 或者在你自己的浏览器里登录一次（封存在数据目录下）
itda accounts / logout <site>            # 查看或移除保存的登录
itda approvals / approve <id> / reject <id>
itda audit [--verify]                    # 代理做了什么
itda check [<site>|--all]                # 每个读取还和站点一致吗？
itda refresh <site>                      # 站点变了：重新起草，保留你决定过的
itda resume <site>                       # 在直接 HTTP 模式下暂停之后，或密钥过期之后
itda feedback                            # 报告问题：打开一个填好的新 issue
```

## 从代理使用

```sh
itda mcp install claude-desktop   # 或 cursor、vscode、codex
itda mcp install claude-code      # 打印 `claude mcp add` 命令
# 加上 --approve-in-client，在客户端自己的表单里批准更改
# itda mcp --site <name> 只提供一个站点，供项目自己的配置使用
```

安装器合并进客户端的配置，保留你的其他服务器。它把旧文件备份为 `.bak`。
`--dry-run` 先显示更改。`itda mcp` 提供每一个已安装的站点：

- 最多 30 个操作各自显示为一个工具（`sandbox.list-notices`），带输入模式
  以及只读、破坏性和幂等提示。
- 超过之后，三个稳定的工具（`itda_search`、`itda_describe`、`itda_call`）
  保持工具列表短小，让提示词缓存继续有效。`--mode` 可以覆盖这一点。
- 结果是和 `itda call --json` 同样的 JSON 信封，标记为不可信。
- 更改返回带 id 的 `approval_required`。没有工具能批准；代理必须来问你。

只有 shell 的代理可以调用 `itda call … --json`，并根据退出码分支：

| 退出码 | 含义 |
|---|---|
| 0 | 成功 |
| 2 | 输入错误或未知操作 |
| 10 | 重新登录（`itda login`） |
| 11 | 页面不再与其定义一致 |
| 12 | 站点拒绝了这次读取，在浏览器里也一样；`paused: false` 表示没有要恢复的 |
| 13 | 被限流 |
| 14 | 站点拒绝了这一个请求，视为不允许；重新登录也没用 |
| 20 | 需要批准，或者用户还没有回答（或者更改已关闭） |
| 21 | 批准已用过、被拒绝、已过期，或因输入变化而撤回 |
| 22 | itda 发送了一个更改，但结果未知；若有 `outcome_check`，更改指定的读取会说明它是否已生效 |
| 30 | 网络错误 |

## 原则

- **本地优先。** 定义在你的设备上运行。凭据从不离开它。
- **默认只读。** 只有当方法、名称、路径、输入和 GraphQL 文档全都一致时，
  一个操作才算读取。其余一切都是更改。
- **人来批准更改。** 每次批准覆盖一个请求，带精确的输入、账户、定义和凭据。
  它只生效一次，然后过期。
- **挑战再试一次，在你的浏览器里。** 站点用挑战页面回应的读取，会在你电脑上
  安装的浏览器里再运行一次；itda 记住哪种方式行得通。更改只发送一次，从不经过
  浏览器。`ITDA_TRANSPORT=http` 则只用直接 HTTP。见
  [docs/HONEST_CLIENT.md](docs/HONEST_CLIENT.md)。
- **经过验证的操作。** 定义声明正确的响应长什么样，所以“已登出”和“站点
  变了”是错误，而不是数据。

## 结构

crate 只依赖它下面的 crate；`cargo xtask check-layers` 强制这一点。

| 层 | Crate | 用途 |
|---|---|---|
| 0 | `itda-types` | 错误契约、URL 检查（公共后缀列表）、脱敏 |
| 1 | `itda-html`、`itda-store` | 不依赖脚本的 HTML 提取；本地 SQLite 存储 |
| 2 | `itda-spec` | 定义格式、请求构建、响应验证 |
| 3 | `itda-policy` | 读/写分类、定义安全检查、批准 |
| 4 | `itda-browser` | 登录、录制、隔离的浏览器读取传输 |
| 5 | `itda-net` | 自适应传输、策略缓存、节流、私有地址防护 |
| 6 | `itda-vault` | 操作系统密钥环或环境中的账户和密钥 |
| 7 | `itda-runtime` | 运行操作：凭据、批准、整形、审计日志 |
| 9 | `itda-setup` | 设置、登录和检查步骤，CLI 和 MCP 服务器共用 |
| 10 | `itda-mcp` | 基于 stdio 的 MCP 服务器 |
| 11 | `apps/cli` | `itda` 命令 |

## 开发

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo xtask check-layers
```

`rust-toolchain.toml` 固定 Rust 1.98（2024 版次）。测试只和它们自己启动的
本地服务器通信。提交和 PR 规则见 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 安全

[docs/THREAT_MODEL.md](docs/THREAT_MODEL.md) 列出了 itda 防御什么、不防御
什么。按 [SECURITY.md](SECURITY.md) 所述报告漏洞。

## 致谢与许可

以 [Apache-2.0](LICENSE) 许可。本项目的部分内容派生自
[Hycli](https://github.com/Hybirdss/Hycli)（Apache-2.0）；见 [NOTICE](NOTICE)。
每个派生文件都在其头部说明了这一点，并描述了改动。本项目与 Hycli 无关，
也不使用其名称或标志。
