<p align="center"><img src="assets/hero.png" alt="itda：一隻手沿著一條線把一張小小的網站卡片遞給 AI 代理。在 itda 後面輸入一個網址，你的 AI 就能使用這個網站。" width="100%"></p>

<p align="center"><strong>在 itda 後面輸入一個網址，你的 AI 就能使用這個網站。</strong></p>

<p align="center"><a href="README.md">English</a> | <a href="README.ko.md">한국어</a> | <a href="README.ja.md">日本語</a> | <a href="README.zh-Hans.md">简体中文</a> | <b>繁體中文</b> | <a href="README.es.md">Español</a> | <a href="README.fr.md">Français</a> | <a href="README.de.md">Deutsch</a> | <a href="README.pt-BR.md">Português (Brasil)</a> | <a href="README.vi.md">Tiếng Việt</a> | <a href="README.id.md">Bahasa Indonesia</a> | <a href="README.ru.md">Русский</a></p>

<p align="center">
  <a href="https://github.com/brnyxx/itda-cli/actions/workflows/ci.yml"><img src="https://github.com/brnyxx/itda-cli/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/brnyxx/itda-cli/releases"><img src="https://img.shields.io/github/v/release/brnyxx/itda-cli?include_prereleases&amp;label=release" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue" alt="License: Apache-2.0"></a>
  <a href="https://brnyxx.github.io/itda-cli/docs/">文件</a>
</p>

給 itda 一個網址。它弄清這個網站能做什麼，並實際試一試。它交給你的
AI 代理一份簡短的、用白話寫的操作清單。讀取直接就能用。在你說「可以」
之前，網站上什麼都不會改。它在你自己的電腦上執行，對需要登入的老式
內部網站同樣有效。

它面向你公司營運的網站和你有帳號的服務，而不是整個網際網路。

<p align="center"><img src="assets/setup.gif" alt="第一次執行：itda 詢問網址，貼上 news.ycombinator.com 之後，它找到一個 30 筆記錄的清單、一個表單和一個訂閱源，把兩個讀取都試了一次，儲存網站，再按一次 Enter 就連接了 Claude Desktop" width="860"></p>

<details>
<summary>同一次執行的文字版</summary>

```console
$ itda news.ycombinator.com
1/5  正在查看 news.ycombinator.com...
    這個頁面不用登入就能開啟。
    找到：一個有 30 筆記錄的清單, 一個表單, 一個訂閱源。
2/5  代理將能做的事：
    讀取   home         List what Hacker News shows  (輸入：p)
    讀取   feed         Read the RSS feed
3/5  正在逐一測試每個讀取操作...
    ✓ home         30 筆記錄
    ✓ feed         成功
4/5  已儲存為 'news-ycombinator'。
    連接 Claude Desktop？[Y/n]
5/5  正在把 itda 加入 Claude Desktop...
已把 itda 加入 Claude Desktop（~/Library/Application Support/Claude/claude_desktop_config.json）。
開啟 Claude Desktop，itda 就在那裡。

完成。請你的代理處理 'news-ycombinator' 上的事項，例如 "用 news-ycombinator List what Hacker News shows"。
```

</details>

對你的代理說：*「用 news-ycombinator 列出 Hacker News 顯示的內容」*。需要
登入表單的網站會改為向你要使用者名稱和密碼。itda 替你填寫網站
自己的表單，並把工作階段留在你的電腦上。如果這樣還不夠（單一登入、一次性
驗證碼），它會開啟你自己的瀏覽器。你照常登入和使用網站，itda 在旁邊觀察。

<p align="center"><img src="assets/flow.svg" alt="在瀏覽器裡錄製一次網站；itda 據此起草指令。代理呼叫它們；變更要等一個人核准。" width="860"></p>

> 狀態：早期，但已用於真實網站。執行階段、CLI、MCP 伺服器、核准（在終端機
> 裡或在代理應用程式內）、稽核日誌、從 OpenAPI 文件、瀏覽器錄製或抓取的
> 頁面起草、API 金鑰、瀏覽器登入和 CSRF 預檢步驟今天都已可用。下一步是
> 簽章的網站套件。見 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)。

## 為什麼

操控瀏覽器的代理每一頁都重新讀，每張截圖或無障礙快照都要花掉數千個
token。頁面叫它點哪裡它就點哪裡，版面一變就悄悄壞掉。itda 把這些工作
搬到了安裝那一步：

| | 操控瀏覽器的代理 | itda |
|---|---|---|
| 代理看到的 | 整頁，每一步 | 一個操作的帶型別結果，上限 24 KB |
| 改版或已登出的頁面 | 當作資料讀進去 | 根據宣告的回應偵測出來（`response_mismatch`、`auth_required`） |
| 網站上的變更 | 模型說了算 | 針對精確輸入的一次性核准，由人在終端機或代理應用程式裡給出 |
| 頁面上寫著「忽略之前的指示」 | 進入提示詞 | 標記為不可信資料，永遠不是工具 |
| 憑證 | 由模型輸入到頁面裡 | 在作業系統鑰匙圈中；代理只看到帳號 id |
| 被封鎖 | 重試、換 IP、冒充瀏覽器 | 在你自己裝好的瀏覽器裡把讀取再試一次，記住哪種方式行得通，什麼都不暫停 |
| 發生了什麼 | 聊天記錄 | 可以驗證的雜湊鏈稽核日誌 |

## 能做什麼，不能做什麼

這裡的每一行都是今天的程式碼真正做到的。`itda setup` 會說明它遇到的是哪
種情況；`itda draft page` 或 `itda record` 會說明它們漏掉了什麼。

**現在就能用**

- 伺服器繪製的頁面：表格、清單和重複的卡片變成記錄，只有一件事物的頁面
  變成對其欄位的讀取。搜尋表單變成讀取，發布表單變成變更，訂閱源一併
  帶上。
- 為指令碼內嵌資料的頁面（`__NEXT_DATA__`、`ld+json`、Apollo 或預載
  狀態）：其中每個清單一個讀取。
- 由指令碼繪製、呼叫回傳 JSON 的 API 的頁面，比如 React 或 Vue 入口網站：
  `itda setup` 在頁面上開啟你的瀏覽器，觀察它自己載入了什麼，並從這些
  呼叫起草。它不點擊也不輸入。
- 需要 CSRF 權杖的表單：預檢步驟在每次核准的送出之前從表單頁面讀取一個
  新的權杖。
- 作為頁框外殼的頁面：內容放在同一網站的 `<iframe>` 裡，往往有自己的
  登入。itda 把每個頁框當作網站的一個頁面來讀取，頁框要求時先登入。
- Cookie 工作階段：透過網站自己的表單用你的使用者名稱和密碼登入，或者用
  你自己的瀏覽器處理單一登入和一次性驗證碼。網址中攜帶的工作階段
  （`?sid=...`）同樣可以。
- 網站發布的 OpenAPI 或 Swagger 說明，以及回應內省的 GraphQL 端點：每個
  操作或查詢欄位一個讀取。
- 執行 GitLab、Jira、Confluence、Redmine、Discourse、MediaWiki、
  WordPress、Gitea 或 Forgejo 的網站：使用其自身的 API，來自現成範本。
- 檔案的接收與傳送：頁面標為附件的連結會變成一個讀取，把檔案儲存到你的電腦並回傳路徑、大小和雜湊，從不回傳位元組。帶檔案控制項的表單會變成一個傳送你指定檔案的變更；你按名稱、大小和雜湊核准它。
- 任何你自己錄製過使用過程的東西，在瀏覽器裡或作為 HAR 檔案。
- 挑戰頁面後面的頁面，只要你電腦上的瀏覽器能過。itda 在那個瀏覽器裡把讀取
  再試一次，視窗隱藏，只給這一個網站和帳號用，並記住哪種方式行得通，下次呼叫
  就從那裡開始。瀏覽器在 CLI 呼叫之間保持執行，閒置五分鐘後關閉；換個搜尋詞
  不會重啟它。頁面載入的一切都經過和直接請求一樣的位址和讀取檢查。見
  [傳輸策略](docs/HONEST_CLIENT.md)。

**錄製後能用**

- 直接抓取時什麼都不顯示、自己也不載入任何東西的頁面。它只在你點擊、
  搜尋或登入之後才去取得。`itda record` 觀察你使用它，並從看到的呼叫
  起草。

**不能用**

- WebSocket。itda 只說普通 HTTP，錄製也不保留 socket 流量。
- 對每個請求簽章的 API（每次呼叫都計算簽章或 nonce）。itda 在鑰匙圈裡
  保存固定的權杖或 API 金鑰。它不會重現每次呼叫都變化的值，所以網站會
  拒絕請求。
- 由 HTML 片段組裝的頁面（htmx 之類）。這不是有意為之：itda 仍然可能
  讀到一個包含整張表格的片段，但讀不到指令碼拿它做了什麼。
- 畫在畫布或圖片裡的內容。itda 讀取 HTML、JSON 和 XML 中的文字；它不看
  像素。
- 你的瀏覽器也過不去的挑戰，或者驗證碼。itda 不解驗證碼。這次讀取失敗並如實
  回報；什麼都不暫停。見 [docs/HONEST_CLIENT.md](docs/HONEST_CLIENT.md)。
- 任何你的帳號看不到的東西。itda 沒有自己的帳號；它用你的。

## 網站變了之後

`itda check <site>` 執行每一個不需要輸入的讀取，從不送出變更。它以一個
結論收尾：`ok`、`signed_out`、`site_changed`、`blocked` 或 `unreachable`。
結束碼隨之而定（`site_changed` 為 11），所以排程工作可以在代理撞上之前
提醒你。在 MCP 上，`itda_check` 回傳同樣的內容。
`itda fsck <site>` 讀取 itda 記下的本機嘗試紀錄（只有結構和判定，沒有值），列出從未嘗試的、需要輸入的以及時而失敗的操作，並附上要執行的指令。

當結論是 `site_changed` 時，`itda refresh <site>` 從當初設定時的來源重新
起草網站：它的頁面、API 說明、GraphQL 端點，或者保留下來的錄製形狀，每個
值都已清空。它逐個操作顯示會改什麼，在你說「可以」之前什麼都不寫。itda
保留你手動編輯過的操作，並更新新草稿改動了的操作。來源不再起草出的操作，
只有在其讀取也失敗時才會被刪掉；每個新操作先試一次，有回應才加入。itda
撤回等待該網站的核准，並在之後立刻再檢查一遍網站。用 `itda add` 安裝的
定義沒有來源，不能重新起草；請手動編輯。

這就是取捨。瀏覽器代理每次執行都在適應，因此慢，而且不確定。itda 是一個
請求、每次同樣的答案；遇到挑戰就在你的瀏覽器裡再試一次，答案仍然一樣。網站
一變，它就大聲報錯。`docs/TRANSPORT_BENCHMARKS.md` 裡有實測數字，包括 itda
還沒達到的那一項。

## 與瀏覽器代理並用

Playwright MCP、browser-use 和電腦操作在每個任務上都操控瀏覽器。itda
不取代它們：只用一次的頁面，或者上面清單裡 itda 不涵蓋的網站，歸它們。
itda 面向代理會反覆呼叫的網站。在瀏覽器裡錄製一次，審閱操作，然後透過
HTTP 執行它們，得到帶型別的回答、每個變更的一次性核准和一份日誌。兩個
都用：用瀏覽器代理探索，用 itda 跑每天都要跑的呼叫。

## 快速入門

```sh
# macOS 和 Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.sh | sh
```

```powershell
# Windows
powershell -ExecutionPolicy Bypass -c "irm https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.ps1 | iex"
```

從 [Releases](https://github.com/brnyxx/itda-cli/releases) 下載壓縮檔，
或者用 `cargo install --git https://github.com/brnyxx/itda-cli itda-cli`
從原始碼建置。

Claude Desktop，不用終端機：下載
[macOS（Apple 晶片）](https://github.com/brnyxx/itda-cli/releases/latest/download/itda-aarch64-apple-darwin.mcpb)、
[macOS（Intel）](https://github.com/brnyxx/itda-cli/releases/latest/download/itda-x86_64-apple-darwin.mcpb) 或
[Windows](https://github.com/brnyxx/itda-cli/releases/latest/download/itda-x86_64-pc-windows-msvc.mcpb) 的套件並開啟檔案。Claude
Desktop 會安裝 itda 及其工具，「use the-site-you-use.example」就能在對話裡
設定一個網站。核准變更仍然在終端機裡用 `itda approve` 完成，所以要做變更
的話也請安裝指令列工具。

輸入 `itda` 和一個你使用的網站網址。它找到你電腦上的代理應用程式，按一次
Enter 就連接好。它會說 12 種語言：由 `LANG` 選擇，`ITDA_LANG` 可以覆寫。只輸入 `itda` 會詢問
網址；`itda setup <address>` 接受供指令碼使用的參數。

itda 能認出執行已知軟體的網站。當頁面說它是 GitLab、Jira、Confluence、
Redmine、Discourse、MediaWiki、WordPress 或 Gitea（也包括 Forgejo），並且
向該軟體自身 API 的一次請求確認了這一點，代理就從現成的範本
（[`sites/software/`](sites/software/)）得到該軟體真正的 API，並按該軟體
的方式登入。頁面本身顯示的內容也會一併加入。`itda setup --as jira` 強制
使用某個範本；`--no-template` 像普通頁面一樣讀取。

由指令碼繪製的頁面只給 itda 看一個標題，所以 itda 會在它上面開啟你的
瀏覽器幾秒鐘。它觀察頁面自己載入了什麼，並從這些呼叫起草。不點擊也不
輸入，視窗會自行關閉。只有仍然什麼都不顯示、或者需要登入的頁面，才會請
你在瀏覽器裡使用網站，由 itda 觀察。

連接好代理之後，你也可以直接對它說：「幫我設定 news.ycombinator.com」。它
呼叫 `itda_setup`。itda 以同樣的方式查看網站、起草並試一試。網站的操作
會作為工具出現在同一個對話裡。指令碼頁面會在你螢幕上的瀏覽器視窗裡短暫
開啟。當某個讀取說需要登入時，`itda_login` 開啟你自己的瀏覽器；代理只
看到帳號 id。

跳過終端機，把下面這段連同你使用的網站網址交給 Claude Code、Cursor 或
Codex：

```text
請為我安裝 itda 並設定 the-site-you-use.example。
1. 安裝：curl -LsSf https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.sh | sh
   （Windows 上：irm https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.ps1 | iex）
2. 執行：itda setup https://the-site-you-use.example --client claude-code -y
   （如果你是 cursor、codex 或 claude-desktop，就用對應的名字）
3. 先閱讀 https://github.com/brnyxx/itda-cli/blob/main/AGENTS.md 這個檔案，然後透過 itda 使用這個網站。
   讀取直接做就行。任何會變更網站的事都要等我核准。
   如果某個讀取說需要登入，告訴我執行：itda login <site> --browser
```

這些環節各自都是獨立的指令。對於不用登入就能開啟的頁面，把它們的網址給
itda。表格、清單和重複的卡片變成記錄。只有一件事物的頁面（一個套件、一篇
文章、一件商品）變成對其標題、內文和帶標籤的值的讀取。連結共享同一形狀
的清單會得到一個按 id 讀取單筆記錄的操作。搜尋表單變成讀取；發布表單變成
變更。訂閱源一併帶上。頁面提到的 GraphQL 端點會被問一次它提供什麼，每個
查詢欄位一個讀取。當網站發布了 API 說明時，它的讀取也加入草稿：

```sh
itda draft page https://example.com/board --install   # 顯示它在頁面上找到了什麼
itda call example.get-board --page 2 --json
itda check example                                     # 頁面還和定義一致嗎？
```

對於需要登入表單的網站，把頁面和你的使用者名稱交給 `itda setup`。它把網站自己的表單送出一次，用的是你輸入的密碼。它把工作階段
保存在系統鑰匙圈裡，並按你登入後看到的頁面起草。定義記住了這個表單，
所以工作階段過期時 `itda login board` 會重新登入：

```sh
itda setup https://intranet.example/board --name board --user me   # 詢問密碼
itda call board.list-notices --json                 # 讀取立刻執行
itda call board.create-notice --title "Server maintenance"   # 變更等待你的核准
itda login board --user me                          # 工作階段過期時
```

對於單一登入或一次性驗證碼，改為錄製你自己使用網站的過程。itda 在一個
新視窗裡開啟你自己的瀏覽器。它觀察你登入、搜尋和送出表單時頁面向伺服器
要了什麼；它從不點擊也不輸入：

```sh
itda record https://intranet.example/board --name board --install   # 完成後按 Enter
                                                    # 然後對保留它用過的工作階段或權杖回答「是」
itda mcp install claude-desktop                     # 或者讓你的代理呼叫它們
```

`examples/lab` 執行 Gitea、Redmine、WordPress、MediaWiki、一個小型舊式內部
網站，以及三個更難看的案例，各自都在登入之後：一個 iframe 裡的公告欄、
一張指令碼從 XHR 繪製的表格、一個帶在 URL 裡的工作階段。一套測試把每個
案例從設定一直走到一次核准的變更。

錄製留在記憶體裡，到達草稿的只有每次呼叫的形狀：沒有標頭、cookie 或你
輸入的值。從瀏覽器開發者工具儲存的 HAR 檔案用 `itda draft har` 的效果
相同。

公開 API 不需要錄製。[sites/](sites/README.md) 裡這些現成的網站內建在
程式裡，不需要帳號：

<p><a href="sites/github.yaml"><img src="assets/brands/github.svg" width="36" height="36" alt="GitHub"></a> <a href="sites/npm.yaml"><img src="assets/brands/npm.svg" width="36" height="36" alt="npm"></a> <a href="sites/crates-io.yaml"><img src="assets/brands/crates-io.svg" width="36" height="36" alt="crates.io"></a> <a href="sites/pypi.yaml"><img src="assets/brands/pypi.svg" width="36" height="36" alt="PyPI"></a></p>

```sh
itda add github      # 或 npm、crates-io、pypi
itda call github.repo --owner rust-lang --repo rust
```

試試內建的沙箱，一個只監聽 127.0.0.1 的假內部網站：按
[examples/sandbox](examples/sandbox/README.md) 操作。一個網站定義就是一個
YAML 檔案。`itda add spec.yaml` 先顯示它把請求送到哪裡、哪些操作會變更
網站，然後安裝它。`itda doctor` 印出資料目錄的位置。
[docs/SPEC.md](docs/SPEC.md) 說明如何撰寫定義。

```sh
itda setup <url>                         # 頁面或錄製 -> 命名的操作 -> 試過 -> 代理，帶提問
itda draft page <url>... [--install]     # 從你使用的頁面起草一個網站
itda record <url> [--install]            # 或者從你在瀏覽器裡的使用過程起草，在登入之後
itda draft har <file> / openapi <file>   # 或者從儲存的錄製或 API 說明起草
itda add spec.yaml / remove <site>       # 安裝或移除一個網站（也可以是 URL 或 github）
itda ls                                  # 已安裝的網站及其操作
itda describe <site>                     # 它們的輸入，以及每個可複製的指令
itda call <site>.<action> --input value  # 執行一個；--json 供代理使用
itda login <site> --user me              # 透過網站自己的表單登入（密碼來自 stdin 或提問）
echo "$KEY" | itda login <site> --secret api-key   # 或者把 API 金鑰存進作業系統鑰匙圈
itda login <site> --browser              # 或者在你自己的瀏覽器裡登入一次（封存在資料目錄下）
itda accounts / logout <site>            # 查看或移除儲存的登入
itda approvals / approve <id> / reject <id>
itda audit [--verify]                    # 代理做了什麼
itda check [<site>|--all]                # 每個讀取還和網站一致嗎？
itda refresh <site>                      # 網站變了：重新起草，保留你決定過的
itda resume <site>                       # 在直接 HTTP 模式下暫停之後，或金鑰過期之後
itda feedback                            # 回報問題：開啟一個填好的新 issue
```

## 從代理使用

```sh
itda mcp install claude-desktop   # 或 cursor、vscode、codex
itda mcp install claude-code      # 印出 `claude mcp add` 指令
# 加上 --approve-in-client，在用戶端自己的表單裡核准變更
# itda mcp --site <name> 只提供一個網站，供專案自己的設定使用
```

安裝程式合併進用戶端的設定，保留你的其他伺服器。它把舊檔案備份為 `.bak`。
`--dry-run` 先顯示變更。`itda mcp` 提供每一個已安裝的網站：

- 最多 30 個操作各自顯示為一個工具（`sandbox.list-notices`），帶輸入結構
  描述以及唯讀、破壞性和冪等提示。
- 超過之後，三個穩定的工具（`itda_search`、`itda_describe`、`itda_call`）
  保持工具清單短小，讓提示詞快取繼續有效。`--mode` 可以覆寫這一點。
- 結果是和 `itda call --json` 同樣的 JSON 信封，標記為不可信。
- 變更回傳帶 id 的 `approval_required`。沒有工具能核准；代理必須來問你。

只有 shell 的代理可以呼叫 `itda call … --json`，並根據結束碼分支：

| 結束碼 | 含義 |
|---|---|
| 0 | 成功 |
| 2 | 輸入錯誤或未知操作 |
| 10 | 重新登入（`itda login`） |
| 11 | 頁面不再與其定義一致 |
| 12 | 網站拒絕了這次讀取，在瀏覽器裡也一樣；`paused: false` 表示沒有要恢復的 |
| 13 | 被限流 |
| 14 | 網站拒絕了這一個請求，視為不允許；重新登入也沒用 |
| 20 | 需要核准，或者使用者還沒有回答（或者變更已關閉） |
| 21 | 核准已用過、被拒絕、已過期，或因輸入變化而撤回 |
| 22 | itda 送出了一個變更，但結果未知；若有 `outcome_check`，變更指定的讀取會說明它是否已生效 |
| 30 | 網路錯誤 |

## 原則

- **本機優先。** 定義在你的裝置上執行。憑證從不離開它。
- **預設唯讀。** 只有當方法、名稱、路徑、輸入和 GraphQL 文件全都一致時，
  一個操作才算讀取。其餘一切都是變更。
- **人來核准變更。** 每次核准涵蓋一個請求，帶精確的輸入、帳號、定義和憑證。
  它只生效一次，然後過期。
- **挑戰再試一次，在你的瀏覽器裡。** 網站用挑戰頁面回應的讀取，會在你電腦上
  安裝的瀏覽器裡再執行一次；itda 記住哪種方式行得通。變更只送出一次，從不經過
  瀏覽器。`ITDA_TRANSPORT=http` 則只用直接 HTTP。見
  [docs/HONEST_CLIENT.md](docs/HONEST_CLIENT.md)。
- **經過驗證的操作。** 定義宣告正確的回應長什麼樣，所以「已登出」和「網站
  變了」是錯誤，而不是資料。

## 結構

crate 只依賴它下面的 crate；`cargo xtask check-layers` 強制這一點。

| 層 | Crate | 用途 |
|---|---|---|
| 0 | `itda-types` | 錯誤契約、URL 檢查（公用後綴清單）、去識別化 |
| 1 | `itda-html`、`itda-store` | 不依賴指令碼的 HTML 擷取；本機 SQLite 儲存 |
| 2 | `itda-spec` | 定義格式、請求建構、回應驗證 |
| 3 | `itda-policy` | 讀/寫分類、定義安全檢查、核准 |
| 4 | `itda-browser` | 登入、錄製、隔離的瀏覽器讀取傳輸 |
| 5 | `itda-net` | 自適應傳輸、策略快取、節流、私有位址防護 |
| 6 | `itda-vault` | 作業系統鑰匙圈或環境中的帳號和金鑰 |
| 7 | `itda-runtime` | 執行操作：憑證、核准、整形、稽核日誌 |
| 9 | `itda-setup` | 設定、登入和檢查步驟，CLI 和 MCP 伺服器共用 |
| 10 | `itda-mcp` | 基於 stdio 的 MCP 伺服器 |
| 11 | `apps/cli` | `itda` 指令 |

## 開發

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo xtask check-layers
```

`rust-toolchain.toml` 固定 Rust 1.98（2024 版次）。測試只和它們自己啟動的
本機伺服器通訊。提交和 PR 規則見 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 安全

[docs/THREAT_MODEL.md](docs/THREAT_MODEL.md) 列出了 itda 防禦什麼、不防禦
什麼。按 [SECURITY.md](SECURITY.md) 所述回報弱點。

## 致謝與授權

以 [Apache-2.0](LICENSE) 授權。本專案的部分內容衍生自
[Hycli](https://github.com/Hybirdss/Hycli)（Apache-2.0）；見 [NOTICE](NOTICE)。
每個衍生檔案都在其標頭說明了這一點，並描述了改動。本專案與 Hycli 無關，
也不使用其名稱或標誌。
