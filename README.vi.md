<p align="center"><img src="assets/hero.png" alt="itda: một bàn tay chuyền một tấm thẻ trang web nhỏ cho agent AI dọc theo một sợi chỉ. Chỉ cần gõ một địa chỉ sau itda và AI của bạn dùng được trang web." width="100%"></p>

<p align="center"><strong>Chỉ cần gõ một địa chỉ sau itda và AI của bạn dùng được trang web.</strong></p>

<p align="center"><a href="README.md">English</a> | <a href="README.ko.md">한국어</a> | <a href="README.ja.md">日本語</a> | <a href="README.zh-Hans.md">简体中文</a> | <a href="README.zh-Hant.md">繁體中文</a> | <a href="README.es.md">Español</a> | <a href="README.fr.md">Français</a> | <a href="README.de.md">Deutsch</a> | <a href="README.pt-BR.md">Português (Brasil)</a> | <b>Tiếng Việt</b> | <a href="README.id.md">Bahasa Indonesia</a> | <a href="README.ru.md">Русский</a></p>

<p align="center">
  <a href="https://github.com/brnyxx/itda-cli/actions/workflows/ci.yml"><img src="https://github.com/brnyxx/itda-cli/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/brnyxx/itda-cli/releases"><img src="https://img.shields.io/github/v/release/brnyxx/itda-cli?include_prereleases&amp;label=release" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue" alt="License: Apache-2.0"></a>
  <a href="https://brnyxx.github.io/itda-cli/docs/">Tài liệu</a>
</p>

Đưa cho itda một địa chỉ. Nó tìm hiểu trang web làm được gì và thử. Nó đưa cho
agent AI của bạn một danh sách ngắn các hành động bằng lời thường. Đọc thì cứ
chạy. Không gì trên trang thay đổi cho đến khi bạn đồng ý. Nó chạy trên chính
máy tính của bạn, và chạy được cả với mạng nội bộ cũ nằm sau trang đăng nhập.

Nó dành cho các trang do công ty bạn vận hành và các dịch vụ bạn có tài khoản,
không phải cả web.

<p align="center"><img src="assets/setup.gif" alt="Lần chạy đầu: itda hỏi địa chỉ, news.ycombinator.com được dán vào, nó tìm thấy một danh sách 30 bản ghi, một biểu mẫu và một feed, thử cả hai lượt đọc, lưu trang web và kết nối Claude Desktop sau một lần nhấn Enter" width="860"></p>

<details>
<summary>Cùng lần chạy đó dưới dạng chữ</summary>

```console
$ itda news.ycombinator.com
1/5  Đang xem news.ycombinator.com...
    Trang này mở được mà không cần đăng nhập.
    Tìm thấy: một danh sách 30 bản ghi, một biểu mẫu, một feed.
2/5  Những gì agent sẽ làm được:
    đọc   home         List what Hacker News shows  (đầu vào: p)
    đọc   feed         Read the RSS feed
3/5  Đang thử từng thao tác đọc một lần...
    ✓ home         30 bản ghi
    ✓ feed         ok
4/5  Đã lưu với tên 'news-ycombinator'.
    Kết nối Claude Desktop? [Y/n]
5/5  Đang thêm itda vào Claude Desktop...
Đã thêm itda vào Claude Desktop (~/Library/Application Support/Claude/claude_desktop_config.json).
Mở Claude Desktop là sẽ thấy itda.

Xong. Hãy hỏi agent của bạn điều gì đó trên 'news-ycombinator', ví dụ "dùng news-ycombinator để List what Hacker News shows".
```

</details>

Hỏi agent của bạn: *"dùng news-ycombinator để liệt kê những gì Hacker News
hiện"*. Một trang web nằm sau biểu mẫu đăng nhập sẽ hỏi tên người dùng và mật
khẩu của bạn thay vào đó. itda điền vào biểu mẫu của chính trang web, dưới tên
của mình, và giữ phiên trên máy tính của bạn. Nếu chưa đủ (đăng nhập một lần,
mã dùng một lần), trình duyệt của chính bạn sẽ mở. Bạn đăng nhập và dùng trang
web như thường trong khi itda quan sát.

<p align="center"><img src="assets/flow.svg" alt="Ghi lại một trang web một lần trong trình duyệt của bạn; itda soạn nháp các lệnh từ đó. Agent gọi chúng; một thay đổi chờ một người phê duyệt." width="860"></p>

> Trạng thái: còn sớm, và đang được dùng với các trang web thật. Runtime, CLI,
> máy chủ MCP, phê duyệt (trong terminal hoặc ngay trong ứng dụng agent), nhật
> ký kiểm toán, soạn nháp từ tài liệu OpenAPI, bản ghi trình duyệt hoặc trang
> đã tải, khóa API, đăng nhập bằng trình duyệt và bước chuẩn bị CSRF đều chạy
> hôm nay. Tiếp theo là các gói trang web có chữ ký. Xem
> [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Vì sao

Các agent điều khiển trình duyệt đọc lại mọi trang và tiêu hàng nghìn token cho
mỗi ảnh chụp màn hình hay ảnh chụp cây trợ năng. Chúng bấm vào bất cứ gì trang
bảo và hỏng âm thầm khi bố cục thay đổi. itda chuyển phần việc đó sang một bước
cài đặt:

| | Agent điều khiển trình duyệt | itda |
|---|---|---|
| Agent thấy gì | Cả trang, ở mỗi bước | Kết quả có kiểu của một hành động, giới hạn 24 KB |
| Trang đã đổi hoặc đã đăng xuất | Đọc như dữ liệu | Phát hiện từ phản hồi đã khai báo (`response_mismatch`, `auth_required`) |
| Thay đổi trên trang web | Tùy mô hình quyết định | Phê duyệt dùng một lần cho đúng đầu vào đó, do một người đưa ra trong terminal hoặc ứng dụng agent |
| Chữ trên trang nói "bỏ qua các chỉ dẫn trước" | Nằm trong prompt | Đánh dấu là dữ liệu không đáng tin, không bao giờ là công cụ |
| Thông tin đăng nhập | Mô hình gõ vào trang | Trong keyring của hệ điều hành; agent chỉ thấy id tài khoản |
| Khi bị chặn | Thử lại, đổi IP, giả làm trình duyệt | Thử lượt đọc thêm một lần trong trình duyệt đã cài trên máy của bạn, nhớ cách đã được, không tạm dừng gì |
| Chuyện gì đã xảy ra | Lịch sử chat | Nhật ký kiểm toán nối chuỗi hash mà bạn kiểm chứng được |

## Làm được gì và không làm được gì

Mọi dòng ở đây đều đúng với mã nguồn hôm nay. `itda setup` nói nó gặp trường
hợp nào; `itda draft page` hoặc `itda record` nói chúng đã bỏ gì.

**Chạy được ngay**

- Trang do máy chủ vẽ: bảng, danh sách và các thẻ lặp lại trở thành bản ghi, và
  một trang về một thứ trở thành một lượt đọc các trường của nó. Biểu mẫu tìm
  kiếm trở thành lượt đọc, biểu mẫu đăng bài trở thành thay đổi, và feed đi kèm.
- Trang nhúng dữ liệu cho script của mình (`__NEXT_DATA__`, `ld+json`, trạng thái
  Apollo hoặc nạp trước): một lượt đọc cho mỗi danh sách trong đó.
- Trang do script vẽ và gọi một API trả JSON, chẳng hạn một cổng React hay Vue:
  `itda setup` mở trình duyệt của bạn tại trang và quan sát nó tự tải những gì.
  Nó soạn nháp từ những lượt gọi đó. Nó không bấm cũng không gõ.
- Biểu mẫu cần token CSRF: một bước chuẩn bị đọc token mới từ trang biểu mẫu ngay
  trước mỗi lần gửi đã được phê duyệt.
- Trang chỉ là vỏ bọc các frame: nội dung nằm trong một `<iframe>` cùng trang
  web, thường sau trang đăng nhập riêng của nó. itda đọc từng frame như một
  trang của trang web và đăng nhập trước khi frame yêu cầu.
- Phiên cookie: đăng nhập qua biểu mẫu của chính trang web với tên người dùng
  và mật khẩu của bạn, hoặc trình duyệt của chính bạn cho đăng nhập một lần và
  mã dùng một lần. Phiên nằm trong địa chỉ (`?sid=...`) cũng chạy theo cách đó.
- Mô tả OpenAPI hoặc Swagger mà trang web công bố, và endpoint GraphQL trả lời
  introspection: một lượt đọc cho mỗi operation hoặc trường truy vấn.
- Trang web chạy GitLab, Jira, Confluence, Redmine, Discourse, MediaWiki,
  WordPress, Gitea hoặc Forgejo: API riêng của chúng, từ một mẫu có sẵn.
- Tệp vào và tệp ra: một liên kết mà trang đánh dấu là tệp đính kèm trở thành
  một lượt đọc lưu tệp trên máy của bạn và trả về đường dẫn, kích thước và
  hash, không bao giờ trả về nội dung. Một biểu mẫu có ô chọn tệp trở thành
  một thay đổi gửi tệp bạn chỉ định; bạn phê duyệt nó theo tên, kích thước và
  hash.
- Bất cứ gì bạn tự ghi lại khi dùng, trong trình duyệt hoặc dưới dạng tệp HAR.
- Trang nằm sau một trang thử thách, khi trình duyệt trên máy tính của bạn vượt
  qua được. itda thử lượt đọc thêm một lần bên trong trình duyệt đó, ẩn, dành
  riêng cho một trang web và tài khoản đó, và nhớ cách đã được, nên lần gọi sau
  bắt đầu từ đó. Trình duyệt giữ ấm giữa các lần gọi CLI và đóng sau năm phút
  không dùng; đổi từ khóa tìm kiếm không khởi động lại nó. Mọi thứ trang tải về
  đều qua cùng các kiểm tra địa chỉ và lượt đọc như một yêu cầu trực tiếp. Xem
  [chính sách truyền tải](docs/HONEST_CLIENT.md).

**Chạy được sau khi bạn ghi lại**

- Trang không hiện gì khi tải thẳng và không tự tải gì. Nó chỉ tải sau khi bạn
  bấm, tìm kiếm hoặc đăng nhập. `itda record` quan sát bạn dùng trang và soạn
  nháp từ những lượt gọi nó thấy.

**Không chạy được**

- WebSocket. itda chỉ nói HTTP thường, và bản ghi không giữ lưu lượng socket.
- API ký từng yêu cầu (một chữ ký hoặc nonce tính cho mỗi lượt gọi). itda giữ
  một token hoặc khóa API cố định trong keyring. Nó không tạo lại được giá trị
  đổi theo từng lượt gọi, nên trang web từ chối yêu cầu.
- Trang ghép từ các mảnh HTML (htmx và tương tự). Không phải do thiết kế: itda
  vẫn có thể đọc một mảnh chứa cả bảng, nhưng không đọc được script làm gì với
  nó.
- Nội dung vẽ trên canvas hoặc trong ảnh. itda đọc chữ trong HTML, JSON và XML;
  nó không nhìn vào pixel.
- Thử thách mà trình duyệt của bạn cũng không vượt qua được, hoặc captcha. itda
  không giải captcha. Lượt đọc thất bại và báo như vậy; không gì bị tạm dừng.
  Xem [docs/HONEST_CLIENT.md](docs/HONEST_CLIENT.md).
- Bất cứ gì tài khoản của bạn không thấy được. itda không có tài khoản riêng;
  nó dùng tài khoản của bạn.

## Khi trang web thay đổi

`itda check <site>` chạy mọi lượt đọc không cần đầu vào và không bao giờ gửi
thay đổi. Nó kết thúc bằng một kết luận: `ok`, `signed_out`, `site_changed`,
`blocked` hoặc `unreachable`. Mã thoát theo kết luận đó (11 cho `site_changed`),
nên một công việc theo lịch có thể cảnh báo bạn trước khi agent vấp phải. Qua
MCP, `itda_check` trả về điều tương tự.
`itda fsck <site>` đọc bản ghi cục bộ mà itda giữ về những gì nó đã thử (chỉ
hình dạng và kết luận, không bao giờ có giá trị) và liệt kê những gì chưa từng
thử, những gì cần đầu vào và những gì thỉnh thoảng thất bại, kèm lệnh cần chạy.

Khi kết luận là `site_changed`, `itda refresh <site>` soạn lại trang web từ
những gì đã dùng để thiết lập: các trang của nó, mô tả API, endpoint GraphQL,
hoặc hình dạng đã giữ của bản ghi với mọi giá trị bị xóa trắng. Nó cho thấy từng
hành động sẽ thay đổi gì và không ghi gì cho đến khi bạn đồng ý. itda giữ các
hành động bạn đã sửa tay và cập nhật các hành động mà bản nháp mới thay đổi. Nó
chỉ bỏ một hành động mà các nguồn không còn soạn ra khi lượt đọc của nó cũng
thất bại, và thử mỗi hành động mới một lần, thêm vào nếu nó trả lời. itda rút
lại các phê duyệt đang chờ cho trang web đó và kiểm tra lại trang web ngay sau
đó. Một định nghĩa cài bằng `itda add` không có nguồn và không soạn lại được;
hãy sửa tay.

Đó là sự đánh đổi. Một agent trình duyệt tự thích nghi mỗi lần chạy, và vì thế
chậm và mỗi lần một khác. itda là một yêu cầu và cùng một câu trả lời mọi lần;
sau một thử thách là thêm một lần thử trong trình duyệt của bạn, và vẫn cùng một
câu trả lời. Nó báo to khi trang web thay đổi. `docs/TRANSPORT_BENCHMARKS.md`
có các con số, kể cả con số itda chưa đạt.

## Bên cạnh các agent trình duyệt

Playwright MCP, browser-use và computer use điều khiển trình duyệt cho mỗi tác
vụ. itda không thay thế chúng: một trang bạn chỉ dùng một lần, hoặc một trang
web trong danh sách trên mà itda không hỗ trợ, là việc của chúng. itda dành cho
trang web mà agent sẽ gọi đi gọi lại. Ghi lại một lần trong trình duyệt, xem qua
các hành động, rồi chạy chúng qua HTTP với câu trả lời có kiểu, một phê duyệt
dùng một lần cho mỗi thay đổi và một nhật ký. Dùng cả hai: agent trình duyệt để
khám phá, itda cho những lượt gọi chạy mỗi ngày.

## Bắt đầu nhanh

```sh
# macOS và Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.sh | sh
```

```powershell
# Windows
powershell -ExecutionPolicy Bypass -c "irm https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.ps1 | iex"
```

Tải một tệp nén từ [Releases](https://github.com/brnyxx/itda-cli/releases),
hoặc build từ mã nguồn bằng `cargo install --git https://github.com/brnyxx/itda-cli itda-cli`.

Claude Desktop, không cần terminal: tải gói cho
[macOS (Apple silicon)](https://github.com/brnyxx/itda-cli/releases/latest/download/itda-aarch64-apple-darwin.mcpb),
[macOS (Intel)](https://github.com/brnyxx/itda-cli/releases/latest/download/itda-x86_64-apple-darwin.mcpb) hoặc
[Windows](https://github.com/brnyxx/itda-cli/releases/latest/download/itda-x86_64-pc-windows-msvc.mcpb) rồi mở tệp. Claude
Desktop cài itda cùng các công cụ của nó, và "dùng the-site-you-use.example"
thiết lập một trang web ngay trong cuộc hội thoại. Việc phê duyệt một thay đổi
vẫn diễn ra trong terminal với `itda approve`, nên để dùng thay đổi hãy cài cả
lệnh.

Gõ `itda` và địa chỉ một trang web bạn dùng. Nó tìm các ứng dụng agent trên máy
tính của bạn và kết nối chúng sau một lần nhấn Enter. Nó nói mười hai ngôn ngữ: `LANG`
chọn một và `ITDA_LANG` ghi đè. `itda` không kèm gì sẽ hỏi địa chỉ; `itda setup <address>` nhận
các cờ cho script.

itda nhận ra các trang web chạy phần mềm quen thuộc. Khi trang nói nó là GitLab,
Jira, Confluence, Redmine, Discourse, MediaWiki, WordPress hoặc Gitea (cả
Forgejo), và một yêu cầu đến API riêng của phần mềm đó xác nhận, agent nhận
được API thật của phần mềm đó từ một mẫu có sẵn
([`sites/software/`](sites/software/)), đăng nhập theo cách phần mềm đó làm.
Những gì chính trang hiện cũng được thêm vào. `itda setup --as jira` buộc dùng
một mẫu; `--no-template` đọc trang như mọi trang khác.

Một trang do script vẽ chỉ cho itda thấy tiêu đề, nên itda mở trình duyệt của
bạn tại trang đó trong vài giây. Nó quan sát trang tự tải những gì và soạn nháp
từ những lượt gọi đó. Không gì bị bấm hay gõ, và cửa sổ tự đóng. Chỉ trang vẫn
không hiện gì, hoặc cần đăng nhập, mới yêu cầu bạn dùng trang web trong trình
duyệt trong khi itda quan sát.

Khi đã kết nối một agent, bạn cũng có thể chỉ cần hỏi nó: "thiết lập
news.ycombinator.com cho tôi". Nó gọi `itda_setup`. itda xem trang web, soạn
nháp và thử theo cùng cách. Các hành động của nó xuất hiện thành công cụ trong
cùng cuộc hội thoại. Một trang do script vẽ mở chốc lát trong cửa sổ trình duyệt
trên màn hình của bạn. Khi một lượt đọc bảo đăng nhập, `itda_login` mở trình
duyệt của chính bạn; agent chỉ thấy id tài khoản.

Bỏ qua terminal và đưa đoạn này cho Claude Code, Cursor hoặc Codex, cùng địa
chỉ một trang web bạn dùng:

```text
Cài itda và thiết lập the-site-you-use.example cho tôi.
1. Cài: curl -LsSf https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.sh | sh
   (trên Windows: irm https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.ps1 | iex)
2. Chạy: itda setup https://the-site-you-use.example --client claude-code -y
   (dùng cursor, codex hoặc claude-desktop nếu bạn là một trong số đó)
3. Đọc https://github.com/brnyxx/itda-cli/blob/main/AGENTS.md, rồi dùng trang web qua itda.
   Đọc thì cứ chạy. Bất cứ gì thay đổi trang web đều phải chờ tôi phê duyệt.
   Nếu một lượt đọc bảo đăng nhập, hãy bảo tôi chạy: itda login <site> --browser
```

Từng phần là những lệnh riêng. Với các trang bạn mở được mà không cần đăng
nhập, đưa địa chỉ của chúng cho itda. Bảng, danh sách và các thẻ lặp lại trở
thành bản ghi. Một trang về một thứ (một gói, một bài viết, một sản phẩm) trở
thành một lượt đọc tiêu đề, nội dung và các giá trị có nhãn của nó. Một danh
sách mà các liên kết có cùng một hình dạng nhận một lượt đọc một bản ghi theo
id. Biểu mẫu tìm kiếm trở thành lượt đọc; biểu mẫu đăng bài trở thành thay đổi.
Feed đi kèm. Một endpoint GraphQL mà trang nêu tên được hỏi một lần xem nó cung
cấp gì, một lượt đọc cho mỗi trường truy vấn. Khi trang web công bố mô tả API,
các lượt đọc của nó được thêm vào bản nháp:

```sh
itda draft page https://example.com/board --install   # cho thấy nó tìm được gì trên trang
itda call example.get-board --page 2 --json
itda check example                                     # trang còn khớp không?
```

Với trang web nằm sau biểu mẫu đăng nhập, đưa cho `itda setup` trang đó và tên
người dùng của bạn. Nó gửi biểu mẫu của chính trang web một lần, dưới tên của
mình, với mật khẩu bạn gõ. Nó giữ phiên trong keyring của hệ thống và soạn nháp
từ trang như bạn thấy khi đã đăng nhập. Định nghĩa nhớ biểu mẫu, nên
`itda login board` đăng nhập lại khi phiên hết hạn:

```sh
itda setup https://intranet.example/board --name board --user me   # hỏi mật khẩu
itda call board.list-notices --json                 # đọc chạy ngay
itda call board.create-notice --title "Server maintenance"   # thay đổi chờ bạn phê duyệt
itda login board --user me                          # khi phiên hết hạn
```

Với đăng nhập một lần hoặc mã dùng một lần, hãy ghi lại cách bạn dùng trang web
thay vào đó. itda mở trình duyệt của chính bạn trong một cửa sổ mới. Nó quan sát
các trang hỏi máy chủ những gì trong khi bạn đăng nhập, tìm kiếm và gửi biểu
mẫu; nó không bao giờ bấm hay gõ:

```sh
itda record https://intranet.example/board --name board --install   # nhấn Enter khi xong
                                                    # rồi đồng ý để giữ phiên hoặc token nó đã dùng
itda mcp install claude-desktop                     # hoặc để agent của bạn gọi chúng
```

`examples/lab` chạy Gitea, Redmine, WordPress, MediaWiki, một mạng nội bộ cũ nhỏ
và ba trường hợp xấu hơn sau trang đăng nhập của chúng: một bảng tin trong
iframe, một bảng do script vẽ từ XHR, và một phiên nằm trong URL. Một bộ kiểm
thử đi qua từng cái từ thiết lập đến một thay đổi đã được phê duyệt.

Bản ghi nằm trong bộ nhớ và chỉ hình dạng của mỗi lượt gọi đến được bản nháp:
không header, cookie hay giá trị bạn đã gõ. Một tệp HAR lưu từ công cụ nhà
phát triển của trình duyệt chạy theo cùng cách với `itda draft har`.

API công khai không cần ghi lại. Các trang web có sẵn này từ
[sites/](sites/README.md) được tích hợp sẵn và không cần tài khoản:

<p><a href="sites/github.yaml"><img src="assets/brands/github.svg" width="36" height="36" alt="GitHub"></a> <a href="sites/npm.yaml"><img src="assets/brands/npm.svg" width="36" height="36" alt="npm"></a> <a href="sites/crates-io.yaml"><img src="assets/brands/crates-io.svg" width="36" height="36" alt="crates.io"></a> <a href="sites/pypi.yaml"><img src="assets/brands/pypi.svg" width="36" height="36" alt="PyPI"></a></p>

```sh
itda add github      # hoặc npm, crates-io, pypi
itda call github.repo --owner rust-lang --repo rust
```

Hãy thử sandbox đi kèm, một mạng nội bộ giả chỉ lắng nghe trên 127.0.0.1: làm
theo [examples/sandbox](examples/sandbox/README.md). Một định nghĩa trang web là
một tệp YAML. `itda add spec.yaml` cài nó sau khi cho thấy nó gửi yêu cầu đến
đâu và hành động nào thay đổi trang web. `itda doctor` in ra vị trí thư mục dữ
liệu. [docs/SPEC.md](docs/SPEC.md) giải thích cách viết một định nghĩa.

```sh
itda setup <url>                         # trang hoặc bản ghi -> hành động có tên -> đã thử -> agent, có hỏi
itda draft page <url>... [--install]     # soạn nháp một trang web từ các trang bạn dùng
itda record <url> [--install]            # hoặc từ việc dùng nó trong trình duyệt, sau trang đăng nhập
itda draft har <file> / openapi <file>   # hoặc từ một bản ghi đã lưu hay một mô tả API
itda add spec.yaml / remove <site>       # cài hoặc gỡ một trang web (cũng nhận URL hoặc github)
itda ls                                  # các trang web đã cài và hành động của chúng
itda describe <site>                     # đầu vào của chúng và một lệnh để chép cho mỗi cái
itda call <site>.<action> --input value  # chạy một cái; --json cho agent
itda login <site> --user me              # đăng nhập qua biểu mẫu của chính trang web (mật khẩu từ stdin hoặc câu hỏi)
echo "$KEY" | itda login <site> --secret api-key   # hoặc cất khóa API trong keyring của hệ điều hành
itda login <site> --browser              # hoặc đăng nhập trong trình duyệt của chính bạn, một lần (niêm phong trong thư mục dữ liệu)
itda accounts / logout <site>            # xem hoặc xóa các đăng nhập đã lưu
itda approvals / approve <id> / reject <id>
itda audit [--verify]                    # agent đã làm gì
itda check [<site>|--all]                # mọi lượt đọc còn khớp với trang web không?
itda refresh <site>                      # trang web đã đổi: soạn lại, giữ những gì bạn đã quyết
itda resume <site>                       # sau khi tạm dừng ở chế độ HTTP trực tiếp, hoặc khóa hết hạn
itda feedback                            # báo lỗi: mở một issue mới, đã điền sẵn
```

## Dùng từ một agent

```sh
itda mcp install claude-desktop   # hoặc cursor, vscode, codex
itda mcp install claude-code      # in ra lệnh `claude mcp add`
# thêm --approve-in-client để phê duyệt thay đổi trong biểu mẫu riêng của client
# itda mcp --site <name> chỉ phục vụ một trang web, cho cấu hình riêng của một dự án
```

Trình cài đặt gộp vào cấu hình của client và giữ các máy chủ khác của bạn. Nó
sao lưu tệp cũ thành `.bak`. `--dry-run` cho thấy thay đổi trước. `itda mcp`
phục vụ mọi trang web đã cài:

- Tối đa 30 hành động xuất hiện thành từng công cụ (`sandbox.list-notices`),
  với schema đầu vào và các gợi ý chỉ đọc, phá hủy, idempotent.
- Nhiều hơn thế, ba công cụ cố định (`itda_search`, `itda_describe`,
  `itda_call`) giữ danh sách công cụ ngắn và cache prompt còn hiệu lực. `--mode`
  ghi đè điều này.
- Kết quả là cùng phong bì JSON như `itda call --json`, được đánh dấu không đáng
  tin.
- Một thay đổi trả về `approval_required` kèm một id. Không công cụ nào phê
  duyệt; agent phải hỏi bạn.

Agent chỉ có shell có thể gọi `itda call … --json` và rẽ nhánh theo mã thoát:

| Mã thoát | Ý nghĩa |
|---|---|
| 0 | ok |
| 2 | đầu vào sai hoặc hành động không rõ |
| 10 | đăng nhập lại (`itda login`) |
| 11 | trang không còn khớp với định nghĩa của nó |
| 12 | trang web đã từ chối lượt đọc này, trong trình duyệt cũng vậy; `paused: false` nghĩa là không có gì để tiếp tục |
| 13 | bị giới hạn tần suất |
| 14 | trang web từ chối riêng yêu cầu này vì không được phép; đăng nhập lại cũng không giúp |
| 20 | cần phê duyệt, hoặc người dùng chưa trả lời (hoặc thay đổi đang tắt) |
| 21 | phê duyệt đã dùng, bị từ chối, hết hạn hoặc bị rút lại vì đầu vào đã đổi |
| 22 | itda đã gửi một thay đổi nhưng không rõ kết quả; với `outcome_check`, lượt đọc mà thay đổi chỉ định cho biết nó đã được áp dụng hay chưa |
| 30 | lỗi mạng |

## Nguyên tắc

- **Cục bộ trước hết.** Định nghĩa chạy trên thiết bị của bạn. Thông tin đăng
  nhập không bao giờ rời khỏi đó.
- **Mặc định chỉ đọc.** Một hành động chỉ là đọc khi phương thức, tên, đường
  dẫn, đầu vào và tài liệu GraphQL của nó đều nhất trí. Mọi thứ khác là thay đổi.
- **Con người phê duyệt thay đổi.** Mỗi phê duyệt áp dụng cho một yêu cầu với
  đúng đầu vào, tài khoản, định nghĩa và thông tin đăng nhập. Nó dùng được một
  lần và hết hạn.
- **Thử thách được thêm một lần thử, trong trình duyệt của bạn.** Lượt đọc mà
  trang web trả về bằng một trang thử thách sẽ chạy thêm một lần bên trong trình
  duyệt đã cài trên máy tính của bạn; itda nhớ cách đã được. Thay đổi được gửi
  một lần và không bao giờ qua trình duyệt. `ITDA_TRANSPORT=http` giữ nguyên chỉ
  HTTP trực tiếp. Xem [docs/HONEST_CLIENT.md](docs/HONEST_CLIENT.md).
- **Hành động được kiểm chứng.** Một định nghĩa khai báo phản hồi đúng trông như
  thế nào, nên "đã đăng xuất" và "trang web đã đổi" là lỗi, không phải dữ liệu.

## Bố cục

Các crate chỉ phụ thuộc vào các crate bên dưới; `cargo xtask check-layers` bắt
buộc điều đó.

| Tầng | Crate | Mục đích |
|---|---|---|
| 0 | `itda-types` | Hợp đồng lỗi, kiểm tra URL (Public Suffix List), che dữ liệu |
| 1 | `itda-html`, `itda-store` | Trích xuất HTML không script; kho SQLite cục bộ |
| 2 | `itda-spec` | Định dạng định nghĩa, dựng yêu cầu, kiểm chứng phản hồi |
| 3 | `itda-policy` | Phân loại đọc/ghi, kiểm tra an toàn định nghĩa, phê duyệt |
| 4 | `itda-browser` | Đăng nhập, bản ghi, truyền tải đọc qua trình duyệt cách ly |
| 5 | `itda-net` | Truyền tải thích ứng, bộ đệm chiến lược, điều tiết, bảo vệ địa chỉ riêng |
| 6 | `itda-vault` | Tài khoản và bí mật trong keyring của hệ điều hành hoặc môi trường |
| 7 | `itda-runtime` | Chạy hành động: thông tin đăng nhập, phê duyệt, định hình, nhật ký kiểm toán |
| 9 | `itda-setup` | Các bước thiết lập, đăng nhập và kiểm tra, dùng chung cho CLI và máy chủ MCP |
| 10 | `itda-mcp` | Máy chủ MCP qua stdio |
| 11 | `apps/cli` | Lệnh `itda` |

## Phát triển

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo xtask check-layers
```

`rust-toolchain.toml` ghim Rust 1.98 (edition 2024). Các bài kiểm thử chỉ nói
chuyện với máy chủ cục bộ mà chúng tự khởi động. Quy tắc commit và PR nằm trong
[CONTRIBUTING.md](CONTRIBUTING.md).

## Bảo mật

[docs/THREAT_MODEL.md](docs/THREAT_MODEL.md) liệt kê những gì itda phòng vệ và
những gì không. Báo lỗ hổng như mô tả trong [SECURITY.md](SECURITY.md).

## Ghi công và giấy phép

Cấp phép theo [Apache-2.0](LICENSE). Một phần dự án này phái sinh từ
[Hycli](https://github.com/Hybirdss/Hycli) (Apache-2.0); xem [NOTICE](NOTICE).
Mỗi tệp phái sinh nói rõ điều đó trong phần đầu và mô tả những gì đã thay đổi.
Dự án này không liên kết với Hycli và không dùng tên hay nhãn hiệu của Hycli.
