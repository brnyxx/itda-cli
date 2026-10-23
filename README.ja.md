<p align="center"><img src="assets/hero.png" alt="itda: 手が小さなウェブサイトのカードを一本の糸に沿って AI エージェントに渡す。itda の後ろにアドレスを打つだけで AI がそのサイトを使えます。" width="100%"></p>

<p align="center"><strong>itda の後ろにアドレスを打つだけで、AI がそのサイトを使えます。</strong></p>

<p align="center"><a href="README.md">English</a> | <a href="README.ko.md">한국어</a> | <b>日本語</b> | <a href="README.zh-Hans.md">简体中文</a> | <a href="README.zh-Hant.md">繁體中文</a> | <a href="README.es.md">Español</a> | <a href="README.fr.md">Français</a> | <a href="README.de.md">Deutsch</a> | <a href="README.pt-BR.md">Português (Brasil)</a> | <a href="README.vi.md">Tiếng Việt</a> | <a href="README.id.md">Bahasa Indonesia</a> | <a href="README.ru.md">Русский</a></p>

<p align="center">
  <a href="https://github.com/brnyxx/itda-cli/actions/workflows/ci.yml"><img src="https://github.com/brnyxx/itda-cli/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/brnyxx/itda-cli/releases"><img src="https://img.shields.io/github/v/release/brnyxx/itda-cli?include_prereleases&amp;label=release" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue" alt="License: Apache-2.0"></a>
  <a href="https://brnyxx.github.io/itda-cli/docs/">ドキュメント</a>
</p>

itda にアドレスを一つ渡してください。サイトでできることを調べ、実際に試します。
AI エージェントには、やさしい言葉で書かれた短いアクション一覧が渡ります。
読み取りはそのまま動きます。あなたが「はい」と言うまでサイトは何も変わりません。
あなたのコンピューターの中だけで動き、サインインが要る古い社内システムでも使えます。

会社が運営するサイトと、あなたがアカウントを持つサービスのための道具です。
ウェブ全体が相手ではありません。

<p align="center"><img src="assets/setup.gif" alt="初回の実行: itda がアドレスを尋ね、news.ycombinator.com を貼り付けると、30 件の一覧とフォーム、フィードを見つけ、二つの読み取りを試し、サイトを保存して Enter 一回で Claude Desktop に接続する" width="860"></p>

<details>
<summary>同じ実行を文字で見る</summary>

```console
$ itda news.ycombinator.com
1/5  news.ycombinator.com を調べています...
    このページはサインインなしで開けます。
    見つかったもの: 30 件の一覧, フォーム 1 つ, フィード。
2/5  エージェントにできるようになること:
    読取   home         List what Hacker News shows  (入力: p)
    読取   feed         Read the RSS feed
3/5  すべての読み取りを一度ずつ試しています...
    ✓ home         30 件
    ✓ feed         ok
4/5  'news-ycombinator' として保存しました。
    Claude Desktop に接続しますか? [Y/n]
5/5  Claude Desktop に itda を追加しています...
Claude Desktop に itda を追加しました (~/Library/Application Support/Claude/claude_desktop_config.json)。
Claude Desktop を開くと itda があります。

完了です。エージェントに 'news-ycombinator' について頼んでみてください。例: "news-ycombinator で List what Hacker News shows"。
```

</details>

エージェントにこう頼みます: *「news-ycombinator で Hacker News の一覧を読んで」*。
サインインフォームの後ろにあるサイトでは、代わりにユーザー名とパスワードを尋ねます。
itda はサイト自身のフォームを itda の名前で送り、セッションをあなたのコンピューターに
保管します。それで足りないとき (シングルサインオン、ワンタイムコード) は、あなたの
ブラウザが開きます。いつもどおりサインインしてサイトを使う間、itda が見守ります。

<p align="center"><img src="assets/flow.svg" alt="ブラウザでウェブサイトを一度記録すると、itda がそこからコマンドの下書きを作ります。エージェントがそれを呼び出し、変更は人の承認を待ちます。" width="860"></p>

> 状況: まだ初期ですが、実際のサイトに対して使われています。ランタイム、CLI、MCP
> サーバー、承認 (端末またはエージェントアプリ内)、監査ログ、OpenAPI ドキュメント、
> ブラウザの記録、取得したページからの下書き、API キー、ブラウザでのサインイン、
> CSRF の事前手順は今日動きます。次は署名付きサイトパックです。
> [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) を参照してください。

## なぜ

ブラウザを操るエージェントはページを毎回読み直し、スクリーンショットや
アクセシビリティスナップショットごとに数千トークンを使います。ページに言われた
ところを何でもクリックし、レイアウトが変わると静かに壊れます。itda はその仕事を
インストール時の一手順に移します:

| | ブラウザを操るエージェント | itda |
|---|---|---|
| エージェントが見るもの | ページ全体を、毎ステップ | アクション一つの型付きの結果、上限 24 KB |
| 変わった、またはサインアウトされたページ | データとして読む | 宣言された応答から検出 (`response_mismatch`、`auth_required`) |
| サイトを変える操作 | モデルが決めたまま | 正確な入力値への一回限りの承認。人が端末かエージェントアプリで与える |
| ページ上の「前の指示は無視して」という文 | プロンプトの中へ | 信頼できないデータと印を付け、ツールにはしない |
| 認証情報 | モデルがページに入力 | OS のキーリングに。エージェントはアカウント id だけを見る |
| ブロックされたとき | 再試行、切り替え、ブラウザを装う | あなたのコンピュータにあるブラウザで読み取りをもう一度試し、うまくいった方法を覚え、何も止めない |
| 何が起きたか | チャットの履歴 | 検証できるハッシュで連なる監査ログ |

## できること、できないこと

ここに書いてあることはすべて、今のコードに当てはまります。`itda setup` はどの
場合に当たったかを言い、`itda draft page` と `itda record` は何を省いたかを言います。

**今動く**

- サーバーで描かれたページ: 表、一覧、繰り返すカードはレコードになり、一つのもの
  を表すページはその項目の読み取りになります。検索フォームは読み取りに、投稿
  フォームは変更になり、フィードも一緒に付いてきます。
- スクリプト向けにデータを埋め込んだページ (`__NEXT_DATA__`、`ld+json`、Apollo や
  事前読み込みの状態): その中の一覧ごとに読み取り一つ。
- JSON を返す API を呼ぶスクリプトで描かれたページ、たとえば React や Vue の
  ポータル: `itda setup` があなたのブラウザでそのページを開き、自分で何を読み込むか
  見守ります。その呼び出しから下書きを作ります。クリックも入力もしません。
- CSRF トークンを求めるフォーム: 承認された送信のたびに、直前の事前手順でフォーム
  ページから新しいトークンを読み取ります。
- フレームの殻になっているページ: 内容は同じサイトの `<iframe>` にあり、しばしば
  別のサインインの後ろにあります。itda は各フレームをサイトのページとして読み、
  フレームが求めれば先にサインインします。
- Cookie のセッション: サイト自身のフォームでユーザー名とパスワードを使った
  サインイン、またはシングルサインオンやワンタイムコードにはあなたのブラウザ。
  アドレスに載ったセッション (`?sid=...`) も同じように動きます。
- サイトが公開する OpenAPI や Swagger の説明、イントロスペクションに答える GraphQL
  エンドポイント: 操作またはクエリフィールドごとに読み取り一つ。
- GitLab、Jira、Confluence、Redmine、Discourse、MediaWiki、WordPress、Gitea、
  Forgejo で動くサイト: 用意されたテンプレートから、それぞれの API を使います。
- ファイルの受け取りと送信: ページが添付ファイルと示すリンクは、そのファイルをあなたのコンピューターに保存してパス、サイズ、ハッシュだけを返す読み取りになります。バイト列は返しません。ファイル入力のあるフォームは、あなたが指定したファイルを送る変更になり、名前、サイズ、ハッシュを見て承認します。
- あなたが自分で使う様子を、ブラウザか HAR ファイルで記録したものなら何でも。
- チャレンジページの後ろにあるページ。あなたのコンピュータのブラウザがそれを
  通れるときです。itda はそのブラウザの中で読み取りをもう一度試します。隠した
  ウィンドウで、そのサイトとアカウントだけのために保ち、うまくいった方法を
  覚えて、次の呼び出しはそこから始めます。ブラウザは CLI の呼び出しの間も
  起動したままで、5 分使わなければ閉じます。検索語を変えても再起動しません。
  ページが読み込むものはすべて、直接のリクエストと同じアドレスと読み取りの
  検査を通ります。[転送ポリシー](docs/HONEST_CLIENT.md) を参照してください。

**記録すれば動く**

- そのまま取得すると何も表示されず、自分では何も読み込まないページ。あなたが
  クリック、検索、サインインしたあとでだけ取得するもの。`itda record` があなたの
  使う様子を見守り、見た呼び出しから下書きを作ります。

**動かない**

- WebSocket。itda は素の HTTP を話し、記録はソケットの通信を残しません。
- リクエストごとに署名する API (呼び出しごとに計算する署名やノンス)。itda は
  固定のトークンや API キーをキーリングに保管します。呼び出しごとに変わる値は
  作り直せないので、サイトはリクエストを拒否します。
- HTML の断片を組み合わせて作るページ (htmx など)。設計上の制限ではありません。
  表全体を含む断片なら itda は読めることもありますが、スクリプトがそれをどう
  扱うかは読めません。
- キャンバスや画像に描かれた内容。itda は HTML、JSON、XML の中の文字を読みます。
  ピクセルは見ません。
- あなたのブラウザでも通れないチャレンジ、または CAPTCHA。itda は CAPTCHA を
  解きません。その読み取りは失敗し、そう伝えます。何も一時停止しません。
  [docs/HONEST_CLIENT.md](docs/HONEST_CLIENT.md) を参照してください。
- あなたのアカウントで見られないもの。itda に自分のアカウントはありません。
  あなたのものを使います。

## サイトが変わったら

`itda check <site>` は入力の要らない読み取りをすべて実行し、変更は決して送りません。
判定一つで終わります: `ok`、`signed_out`、`site_changed`、`blocked`、`unreachable`。
終了コードはそれに従う (`site_changed` なら 11) ので、定期実行のジョブがエージェント
のつまずく前に警告できます。MCP 経由では `itda_check` が同じものを返します。
`itda fsck <site>` は、itda が試したことを書き留めたローカルの記録（形と判定だけで、値は含みません）を読み、まだ試していないもの、入力が必要なもの、ときどき失敗するものを、実行するコマンドと一緒に示します。

判定が `site_changed` なら、`itda refresh <site>` がセットアップに使った出どころから
サイトを作り直します: ページ、API 説明、GraphQL エンドポイント、または値をすべて
消して形だけ残した記録。アクションごとに何が変わるかを示し、あなたが「はい」と
言うまで何も書きません。手で直したアクションは残し、新しい下書きで変わった
アクションは更新します。出どころからもう作られないアクションは、その読み取りも
失敗したときだけ外し、新しいアクションは一度試して、答えが返れば加えます。
itda はそのサイトで待っている承認を取り下げ、直後にサイトをもう一度点検します。
`itda add` で入れた定義には出どころがないので作り直せません。手で編集してください。

これがトレードオフです。ブラウザエージェントは実行のたびに合わせますが、そのぶん
遅く、毎回結果が違います。itda はリクエスト一つに毎回同じ答えです。チャレンジの
あとはあなたのブラウザでもう一度試しますが、答えはやはり同じです。サイトが動けば
大きな音を立てて壊れます。`docs/TRANSPORT_BENCHMARKS.md` に数値があり、itda が
まだ満たしていないものも載せています。

## ブラウザエージェントの隣で

Playwright MCP、browser-use、コンピューター操作はタスクごとにブラウザを操ります。
itda はそれらの代わりではありません。一度しか使わないページや、上の一覧にある
itda が扱えないサイトはそちらの出番です。itda はエージェントが何度も呼ぶサイトの
ためのものです。ブラウザで一度記録し、アクションを見わたし、あとは HTTP で、型付きの
答え、変更ごとの一回限りの承認、ログとともに実行します。両方使ってください。
探索はブラウザエージェントで、毎日走る呼び出しは itda で。

## クイックスタート

```sh
# macOS と Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.sh | sh
```

```powershell
# Windows
powershell -ExecutionPolicy Bypass -c "irm https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.ps1 | iex"
```

[Releases](https://github.com/brnyxx/itda-cli/releases) からアーカイブを
ダウンロードするか、`cargo install --git https://github.com/brnyxx/itda-cli itda-cli`
でソースからビルドします。

Claude Desktop で、端末なしで使うには: バンドルを
[macOS (Apple silicon)](https://github.com/brnyxx/itda-cli/releases/latest/download/itda-aarch64-apple-darwin.mcpb)、
[macOS (Intel)](https://github.com/brnyxx/itda-cli/releases/latest/download/itda-x86_64-apple-darwin.mcpb)、
[Windows](https://github.com/brnyxx/itda-cli/releases/latest/download/itda-x86_64-pc-windows-msvc.mcpb) のどれかでダウンロードして、
ファイルを開きます。Claude Desktop が itda とそのツールをインストールし、
「the-site-you-use.example を使って」と言えば会話の中からサイトをセットアップ
します。変更の承認は引き続き端末の `itda approve` で行うので、変更を使うなら
コマンドもインストールしてください。

`itda` と、あなたが使うサイトのアドレスを打ちます。コンピューター上のエージェント
アプリを見つけ、Enter 一回で接続します。12 の言語を話します。`LANG` で選ばれ、`ITDA_LANG` があればそれが優先されます。
`itda` だけならアドレスを尋ね、`itda setup <address>` はスクリプト向けの
フラグを受け付けます。

itda は既知のソフトウェアで動くサイトを認識します。ページが GitLab、Jira、
Confluence、Redmine、Discourse、MediaWiki、WordPress、Gitea (Forgejo も) だと
言い、そのソフトウェア自身の API への一回のリクエストで確認が取れると、
エージェントは用意されたテンプレート ([`sites/software/`](sites/software/)) から
そのソフトウェアの本物の API を、そのソフトウェアのやり方でサインインして
手に入れます。ページ自身が見せるものもそこに加わります。`itda setup --as jira` は
テンプレートを強制し、`--no-template` はページを他と同じように読みます。

スクリプトで描かれたページは itda にタイトルしか見せないので、itda はあなたの
ブラウザでそのページを数秒だけ開きます。ページが自分で何を読み込むかを見守り、
その呼び出しから下書きを作ります。クリックも入力もせず、ウィンドウは自動で閉じ
ます。それでも何も表示されないページや、サインインが要るページだけが、itda の
見守る中であなたにブラウザでサイトを使うよう求めます。

エージェントを接続したら、こう頼むだけでもかまいません: 「news.ycombinator.com を
セットアップして」。エージェントが `itda_setup` を呼びます。itda は同じようにサイト
を調べ、下書きを作り、試します。そのアクションは同じ会話の中でツールとして現れ
ます。スクリプトで描かれたページは、あなたの画面でブラウザのウィンドウが短く
開きます。読み取りがサインインを求めると、`itda_login` があなたのブラウザを開き
ます。エージェントが見るのはアカウント id だけです。

端末を飛ばして、あなたが使うサイトのアドレスとともに、これを Claude Code、
Cursor、Codex に渡してください:

```text
itda をインストールして the-site-you-use.example をセットアップして。
1. インストール: curl -LsSf https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.sh | sh
   (Windows なら: irm https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.ps1 | iex)
2. 実行: itda setup https://the-site-you-use.example --client claude-code -y
   (あなたが cursor、codex、claude-desktop ならその名前で)
3. https://github.com/brnyxx/itda-cli/blob/main/AGENTS.md を読んでから、itda でそのサイトを使って。
   読み取りはそのまま実行していい。サイトを変える操作は私の承認を待って。
   読み取りがサインインを求めたら、私にこれを実行するよう伝えて: itda login <site> --browser
```

それぞれの部品は独立したコマンドです。サインインなしで開けるページなら、itda に
そのアドレスを渡します。表、一覧、繰り返すカードはレコードになります。一つのもの
(パッケージ、記事、商品) を表すページは、見出し、本文、ラベル付きの値の読み取りに
なります。リンクが同じ形をした一覧には、id で一件を読む読み取りが付きます。検索
フォームは読み取りに、投稿フォームは変更になります。フィードも付いてきます。ページ
が名前を挙げる GraphQL エンドポイントには何を提供しているか一度尋ね、クエリ
フィールドごとに読み取り一つを作ります。サイトが API 説明を公開していれば、その
読み取りも下書きに加わります:

```sh
itda draft page https://example.com/board --install   # ページで見つけたものを表示する
itda call example.get-board --page 2 --json
itda check example                                     # ページはまだ合っているか?
```

サインインフォームの後ろにあるサイトでは、`itda setup` にそのページとユーザー名を
渡します。あなたが打ったパスワードで、サイト自身のフォームを itda の名前で一度
送ります。セッションはシステムのキーリングに保管し、サインインして見えるとおりの
ページから下書きを作ります。定義はフォームを覚えているので、セッションが切れたら
`itda login board` でもう一度サインインします:

```sh
itda setup https://intranet.example/board --name board --user me   # パスワードを尋ねる
itda call board.list-notices --json                 # 読み取りはすぐ動く
itda call board.create-notice --title "Server maintenance"   # 変更はあなたの承認を待つ
itda login board --user me                          # セッションが切れたら
```

シングルサインオンやワンタイムコードには、代わりに自分が使う様子を記録します。
itda はあなたのブラウザを新しいウィンドウで開きます。あなたがサインインし、検索し、
フォームを送る間、ページがサーバーに何を求めるかを見守ります。クリックも入力も
しません:

```sh
itda record https://intranet.example/board --name board --install   # 終わったら Enter
                                                    # 使ったセッションやトークンを残すか「はい」で答える
itda mcp install claude-desktop                     # またはエージェントに呼ばせる
```

`examples/lab` は Gitea、Redmine、WordPress、MediaWiki、小さな古い社内システム、
そしてサインインの後ろにある三つのやっかいな例を動かします: iframe の中の掲示板、
スクリプトが XHR から描く表、URL に載ったセッション。テストスイートがそれぞれを
セットアップから承認された変更まで歩きます。

記録はメモリの中に留まり、下書きに届くのは各呼び出しの形だけです。ヘッダー、
Cookie、あなたが打った値は届きません。ブラウザの開発者ツールから保存した HAR
ファイルも `itda draft har` で同じように使えます。

公開 API には記録は要りません。[sites/](sites/README.md) にある用意済みのサイトは
組み込みで、アカウントなしで使えます:

<p><a href="sites/github.yaml"><img src="assets/brands/github.svg" width="36" height="36" alt="GitHub"></a> <a href="sites/npm.yaml"><img src="assets/brands/npm.svg" width="36" height="36" alt="npm"></a> <a href="sites/crates-io.yaml"><img src="assets/brands/crates-io.svg" width="36" height="36" alt="crates.io"></a> <a href="sites/pypi.yaml"><img src="assets/brands/pypi.svg" width="36" height="36" alt="PyPI"></a></p>

```sh
itda add github      # または npm、crates-io、pypi
itda call github.repo --owner rust-lang --repo rust
```

同梱のサンドボックス、127.0.0.1 だけで聞いている偽の社内システムを試すには
[examples/sandbox](examples/sandbox/README.md) に従ってください。サイト定義は YAML
ファイル一つです。`itda add spec.yaml` は、どこにリクエストを送り、どのアクションが
サイトを変えるかを示したあとでインストールします。`itda doctor` はデータ
ディレクトリの場所を表示します。定義の書き方は [docs/SPEC.md](docs/SPEC.md) に
あります。

```sh
itda setup <url>                         # ページか記録 -> 名前付きアクション -> 試す -> エージェント、質問つきで
itda draft page <url>... [--install]     # あなたが使うページからサイトの下書きを作る
itda record <url> [--install]            # またはブラウザで使う様子から、サインインの後ろでも
itda draft har <file> / openapi <file>   # または保存した記録や API 説明から
itda add spec.yaml / remove <site>       # サイトを入れる、外す (URL や github も)
itda ls                                  # 入っているサイトとそのアクション
itda describe <site>                     # その入力と、それぞれコピーできるコマンド
itda call <site>.<action> --input value  # 一つ実行する。エージェントには --json
itda login <site> --user me              # サイト自身のフォームでサインイン (パスワードは stdin か質問で)
echo "$KEY" | itda login <site> --secret api-key   # または API キーを OS のキーリングに保管
itda login <site> --browser              # または自分のブラウザで一度サインイン (データディレクトリの中に封印)
itda accounts / logout <site>            # 保存したサインインを見る、消す
itda approvals / approve <id> / reject <id>
itda audit [--verify]                    # エージェントが何をしたか
itda check [<site>|--all]                # すべての読み取りはまだサイトに合っているか?
itda refresh <site>                      # サイトが変わった: 作り直し、あなたの決めたことは残す
itda resume <site>                       # 直接 HTTP モードで一時停止したあと、または鍵が切れたあとに
itda feedback                            # 問題を報告する: 記入済みの新しいイシューを開く
```

## エージェントから使う

```sh
itda mcp install claude-desktop   # または cursor、vscode、codex
itda mcp install claude-code      # `claude mcp add` コマンドを表示する
# --approve-in-client を付けると、クライアント自身のフォームで変更を承認できる
# itda mcp --site <name> は一つのサイトだけを提供する。プロジェクト固有の設定向け
```

インストーラーはクライアントの設定に書き足し、他のサーバーはそのまま残します。
古いファイルは `.bak` としてバックアップします。`--dry-run` は変更を先に表示します。
`itda mcp` は入っているすべてのサイトを提供します:

- アクションが 30 個までなら、それぞれがツール一つ (`sandbox.list-notices`) として
  現れ、入力スキーマと読み取り専用・破壊的・冪等のヒントが付きます。
- それより多ければ、固定のツール三つ (`itda_search`、`itda_describe`、`itda_call`)
  がツール一覧を短く保ち、プロンプトキャッシュを有効なままにします。`--mode` で
  上書きできます。
- 結果は `itda call --json` と同じ JSON エンベロープで、信頼できない印が付きます。
- 変更は id 付きの `approval_required` を返します。承認するツールはありません。
  エージェントはあなたに頼むしかありません。

シェルしかないエージェントは `itda call … --json` を呼び、終了コードで分岐できます:

| 終了コード | 意味 |
|---|---|
| 0 | ok |
| 2 | 不正な入力、または未知のアクション |
| 10 | もう一度サインイン (`itda login`) |
| 11 | ページが定義に合わなくなった |
| 12 | サイトがこの読み取りを拒否した。ブラウザでも同じ。`paused: false` なら再開するものはない |
| 13 | レート制限 |
| 14 | サイトがこの一回のリクエストを許可しないとして拒否した。サインインし直しても解決しない |
| 20 | 承認が必要、またはユーザーがまだ答えていない (または変更が無効) |
| 21 | 承認が使用済み、拒否、期限切れ、または入力が変わったので取り下げられた |
| 22 | itda は変更を送ったが結果が不明。`outcome_check` があれば、変更が指定した読み取りが反映の有無を示す |
| 30 | ネットワークエラー |

## 原則

- **ローカル第一。** 定義はあなたの端末で動きます。認証情報はそこから出ません。
- **既定は読み取りのみ。** メソッド、名前、パス、入力、GraphQL ドキュメントが
  すべて一致するときだけ、アクションは読み取りです。それ以外はすべて変更です。
- **変更は人が承認する。** 承認一つは、正確な入力、アカウント、定義、認証情報を
  持つリクエスト一つをカバーします。一度だけ使え、期限が切れます。
- **チャレンジにはもう一度、あなたのブラウザで。** サイトがチャレンジページで
  答えた読み取りは、あなたのコンピュータに入っているブラウザの中でもう一度
  実行します。itda はうまくいった方法を覚えます。変更は一度だけ送り、ブラウザ
  経由では送りません。`ITDA_TRANSPORT=http` なら直接 HTTP だけを使います。
  [docs/HONEST_CLIENT.md](docs/HONEST_CLIENT.md) を参照してください。
- **検証されたアクション。** 定義は正しい応答の形を宣言するので、「サインアウト
  された」や「サイトが変わった」はデータではなくエラーです。

## 構成

クレートは自分より下のクレートだけに依存します。`cargo xtask check-layers` が
それを守らせます。

| 層 | クレート | 役割 |
|---|---|---|
| 0 | `itda-types` | エラーの契約、URL の検査 (Public Suffix List)、秘匿処理 |
| 1 | `itda-html`、`itda-store` | スクリプトなしの HTML 抽出。ローカルの SQLite ストア |
| 2 | `itda-spec` | 定義の形式、リクエストの構築、応答の検証 |
| 3 | `itda-policy` | 読み取り/書き込みの分類、定義の安全性検査、承認 |
| 4 | `itda-browser` | サインイン、記録、隔離されたブラウザ読み取り転送 |
| 5 | `itda-net` | 適応的な転送、戦略キャッシュ、間隔調整、プライベートアドレスの防護 |
| 6 | `itda-vault` | OS のキーリングまたは環境にあるアカウントと秘密 |
| 7 | `itda-runtime` | アクションの実行: 認証情報、承認、整形、監査ログ |
| 9 | `itda-setup` | セットアップ、サインイン、点検の手順。CLI と MCP サーバーで共通 |
| 10 | `itda-mcp` | stdio 上の MCP サーバー |
| 11 | `apps/cli` | `itda` コマンド |

## 開発

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo xtask check-layers
```

`rust-toolchain.toml` は Rust 1.98 (edition 2024) を固定します。テストは自分で起動
したローカルサーバーとだけ通信します。コミットとプルリクエストの規則は
[CONTRIBUTING.md](CONTRIBUTING.md) にあります。

## セキュリティ

[docs/THREAT_MODEL.md](docs/THREAT_MODEL.md) は itda が何を防ぎ、何を防がないかを
挙げています。脆弱性は [SECURITY.md](SECURITY.md) の説明どおりに報告してください。

## クレジットとライセンス

[Apache-2.0](LICENSE) の下で提供します。このプロジェクトの一部は
[Hycli](https://github.com/Hybirdss/Hycli) (Apache-2.0) から派生しています。
[NOTICE](NOTICE) を参照してください。派生した各ファイルはヘッダーでそのことを
述べ、何を変えたかを説明します。このプロジェクトは Hycli と提携しておらず、その
名前や商標を使いません。
