//! Descriptions a person reads: what an action does, in their language,
//! built from what the page or the call showed rather than from its path.
//! The action id stays machine-shaped (`list-notices`); this is the `desc`
//! an agent and the person both see.
use itda_types::lang::LANGUAGES;

/// What an action does, with the subject it does it to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Label<'a> {
    /// Read a page by its title.
    Page { title: &'a str },
    /// Read the records a page lists.
    List { title: &'a str },
    /// Read the data a page embeds for its scripts.
    Embedded { title: &'a str },
    /// Read one kind of record a page embeds, named in plain words
    /// (`news articles`, `products`).
    Records { what: &'a str, title: &'a str },
    /// Read one of the records a page lists, by its id.
    One { what: &'a str, title: &'a str },
    /// A search form on a page.
    Search { title: &'a str },
    /// A form that changes the site.
    Submit { title: &'a str },
    /// A form that sends a file.
    Upload { title: &'a str },
    /// A file the page links to as an attachment.
    Attachment { title: &'a str },
    /// An RSS or Atom feed.
    Feed { title: &'a str },
    /// An API call, by the verb its name starts with and the resource.
    Api { verb: &'a str, resource: &'a str },
    /// A GraphQL operation.
    Graphql { name: &'a str, mutation: bool },
}

/// The language to describe actions in: the one the person set, else the
/// page's own, else English. Any of [`LANGUAGES`].
pub fn language(options: &str, page_lang: &str) -> String {
    [options, page_lang]
        .into_iter()
        .find_map(itda_types::lang::supported)
        .unwrap_or("en")
        .into()
}

/// One line, 80 characters at most, in `lang` (one of [`LANGUAGES`];
/// English for anything else). The site's own words (titles, names) are
/// kept as the site wrote them.
pub fn describe(label: &Label<'_>, lang: &str) -> String {
    let column = LANGUAGES.iter().position(|tag| *tag == lang).unwrap_or(0);
    let template = |key: &str| {
        TEMPLATES
            .iter()
            .find(|(name, _)| *name == key)
            .map_or("", |(_, row)| row[column])
    };
    let subject = |title: &str| {
        if title.is_empty() {
            template("the_page").to_owned()
        } else {
            title.to_owned()
        }
    };
    let what = |what: &'_ str| {
        if lang == "ko" {
            korean_what(what).to_owned()
        } else {
            what.to_owned()
        }
    };
    let (key, fields): (&str, Vec<(&str, String)>) = match label {
        Label::Page { title } => ("page", vec![("title", subject(title))]),
        Label::List { title } => ("list", vec![("title", subject(title))]),
        Label::Embedded { title } => ("embedded", vec![("title", subject(title))]),
        Label::Records { what: kind, title } => (
            "records",
            vec![("what", what(kind)), ("title", subject(title))],
        ),
        Label::One { what: kind, title } => {
            ("one", vec![("what", what(kind)), ("title", subject(title))])
        }
        Label::Search { title } => ("search", vec![("title", subject(title))]),
        Label::Submit { title } => ("submit", vec![("title", subject(title))]),
        Label::Upload { title } => ("upload", vec![("title", subject(title))]),
        Label::Attachment { title } => ("attachment", vec![("title", subject(title))]),
        Label::Feed { title: "" } => ("site_feed", Vec::new()),
        Label::Feed { title } => ("feed", vec![("title", (*title).to_owned())]),
        Label::Api { verb, resource } => (
            match *verb {
                "list" => "api_list",
                "get" => "api_get",
                "create" => "api_create",
                "update" => "api_update",
                "delete" => "api_delete",
                "search" => "api_search",
                _ => "api_send",
            },
            vec![("resource", resource.replace(['-', '_'], " "))],
        ),
        Label::Graphql { name, mutation } => (
            if *mutation {
                "graphql_mutation"
            } else {
                "graphql_query"
            },
            vec![("name", words(name))],
        ),
    };
    let mut text = template(key).to_owned();
    for (name, value) in fields {
        text = text.replace(&format!("{{{name}}}"), &value);
    }
    text.chars().take(80).collect()
}

/// The phrases a description is built from, one row per phrase and one
/// column per language in the order of [`LANGUAGES`]. `{title}`, `{what}`,
/// `{resource}` and `{name}` are filled in with the site's own words;
/// `the_page` stands in for a page without a title. Every language keeps the
/// fields English has (a test checks that), in whatever order its grammar
/// wants.
const TEMPLATES: &[(&str, [&str; 12])] = &[
    (
        "the_page",
        [
            "the page",
            "페이지",
            "ページ",
            "页面",
            "頁面",
            "la página",
            "la page",
            "die Seite",
            "a página",
            "trang",
            "halaman",
            "страница",
        ],
    ),
    (
        "page",
        [
            "Read {title}",
            "{title} 읽기",
            "{title} を読む",
            "读取 {title}",
            "讀取 {title}",
            "Leer {title}",
            "Lire {title}",
            "{title} lesen",
            "Ler {title}",
            "Đọc {title}",
            "Baca {title}",
            "Прочитать {title}",
        ],
    ),
    (
        "list",
        [
            "List what {title} shows",
            "{title} 목록 읽기",
            "{title} の一覧を読む",
            "列出 {title} 显示的内容",
            "列出 {title} 顯示的內容",
            "Listar lo que muestra {title}",
            "Lister ce que montre {title}",
            "Auflisten, was {title} zeigt",
            "Listar o que {title} mostra",
            "Liệt kê những gì {title} hiển thị",
            "Daftar yang ditampilkan {title}",
            "Список того, что показывает {title}",
        ],
    ),
    (
        "embedded",
        [
            "Read the data behind {title}",
            "{title} 데이터 읽기",
            "{title} のデータを読む",
            "读取 {title} 背后的数据",
            "讀取 {title} 背後的資料",
            "Leer los datos detrás de {title}",
            "Lire les données derrière {title}",
            "Die Daten hinter {title} lesen",
            "Ler os dados por trás de {title}",
            "Đọc dữ liệu phía sau {title}",
            "Baca data di balik {title}",
            "Прочитать данные, стоящие за {title}",
        ],
    ),
    (
        "records",
        [
            "List the {what} on {title}",
            "{title}의 {what} 목록 읽기",
            "{title} の {what} の一覧を読む",
            "列出 {title} 上的 {what}",
            "列出 {title} 上的 {what}",
            "Listar {what} en {title}",
            "Lister les {what} sur {title}",
            "Die {what} auf {title} auflisten",
            "Listar {what} em {title}",
            "Liệt kê {what} trên {title}",
            "Daftar {what} di {title}",
            "Список {what} на {title}",
        ],
    ),
    (
        "one",
        [
            "Read one of the {what} on {title}",
            "{title}의 {what} 하나 읽기",
            "{title} の {what} を1件読む",
            "读取 {title} 上的一条 {what}",
            "讀取 {title} 上的一筆 {what}",
            "Leer uno de {what} en {title}",
            "Lire l'un des {what} sur {title}",
            "Einen Eintrag der {what} auf {title} lesen",
            "Ler um de {what} em {title}",
            "Đọc một mục {what} trên {title}",
            "Baca satu {what} di {title}",
            "Прочитать одну запись {what} на {title}",
        ],
    ),
    (
        "search",
        [
            "Search on {title}",
            "{title}에서 검색",
            "{title} で検索",
            "在 {title} 上搜索",
            "在 {title} 上搜尋",
            "Buscar en {title}",
            "Rechercher sur {title}",
            "Auf {title} suchen",
            "Pesquisar em {title}",
            "Tìm kiếm trên {title}",
            "Cari di {title}",
            "Поиск на {title}",
        ],
    ),
    (
        "submit",
        [
            "Submit the form on {title}",
            "{title}의 폼 보내기",
            "{title} のフォームを送信",
            "提交 {title} 上的表单",
            "送出 {title} 上的表單",
            "Enviar el formulario de {title}",
            "Envoyer le formulaire de {title}",
            "Das Formular auf {title} absenden",
            "Enviar o formulário de {title}",
            "Gửi biểu mẫu trên {title}",
            "Kirim formulir di {title}",
            "Отправить форму на {title}",
        ],
    ),
    (
        "upload",
        [
            "Upload a file to {title}",
            "{title}에 파일 올리기",
            "{title} にファイルをアップロード",
            "上传文件到 {title}",
            "上傳檔案到 {title}",
            "Subir un archivo a {title}",
            "Envoyer un fichier à {title}",
            "Eine Datei auf {title} hochladen",
            "Enviar um arquivo para {title}",
            "Tải một tệp lên {title}",
            "Unggah file ke {title}",
            "Загрузить файл на {title}",
        ],
    ),
    (
        "attachment",
        [
            "Download an attachment from {title}",
            "{title}의 첨부 파일 내려받기",
            "{title} から添付ファイルをダウンロード",
            "从 {title} 下载附件",
            "從 {title} 下載附件",
            "Descargar un adjunto de {title}",
            "Télécharger une pièce jointe de {title}",
            "Einen Anhang von {title} herunterladen",
            "Baixar um anexo de {title}",
            "Tải tệp đính kèm từ {title}",
            "Unduh lampiran dari {title}",
            "Скачать вложение с {title}",
        ],
    ),
    (
        "feed",
        [
            "Read the {title} feed",
            "{title} 피드 읽기",
            "{title} のフィードを読む",
            "读取 {title} 订阅源",
            "讀取 {title} 訂閱源",
            "Leer el feed de {title}",
            "Lire le flux {title}",
            "Den Feed {title} lesen",
            "Ler o feed {title}",
            "Đọc nguồn cấp {title}",
            "Baca feed {title}",
            "Прочитать ленту {title}",
        ],
    ),
    (
        "site_feed",
        [
            "Read the site's feed",
            "최신 글 피드 읽기",
            "サイトのフィードを読む",
            "读取站点的订阅源",
            "讀取網站的訂閱源",
            "Leer el feed del sitio",
            "Lire le flux du site",
            "Den Feed der Site lesen",
            "Ler o feed do site",
            "Đọc nguồn cấp của trang web",
            "Baca feed situs",
            "Прочитать ленту сайта",
        ],
    ),
    (
        "api_list",
        [
            "List {resource}",
            "{resource} 목록 가져오기",
            "{resource} の一覧を取得",
            "列出 {resource}",
            "列出 {resource}",
            "Listar {resource}",
            "Lister {resource}",
            "{resource} auflisten",
            "Listar {resource}",
            "Liệt kê {resource}",
            "Daftar {resource}",
            "Список {resource}",
        ],
    ),
    (
        "api_get",
        [
            "Get one of {resource}",
            "{resource} 하나 가져오기",
            "{resource} を1件取得",
            "获取一条 {resource}",
            "取得一筆 {resource}",
            "Obtener uno de {resource}",
            "Obtenir un élément de {resource}",
            "Einen Eintrag von {resource} abrufen",
            "Obter um de {resource}",
            "Lấy một mục {resource}",
            "Ambil satu {resource}",
            "Получить одну запись {resource}",
        ],
    ),
    (
        "api_create",
        [
            "Create {resource}",
            "{resource} 만들기",
            "{resource} を作成",
            "创建 {resource}",
            "建立 {resource}",
            "Crear {resource}",
            "Créer {resource}",
            "{resource} anlegen",
            "Criar {resource}",
            "Tạo {resource}",
            "Buat {resource}",
            "Создать {resource}",
        ],
    ),
    (
        "api_update",
        [
            "Update {resource}",
            "{resource} 고치기",
            "{resource} を更新",
            "更新 {resource}",
            "更新 {resource}",
            "Actualizar {resource}",
            "Modifier {resource}",
            "{resource} aktualisieren",
            "Atualizar {resource}",
            "Cập nhật {resource}",
            "Perbarui {resource}",
            "Обновить {resource}",
        ],
    ),
    (
        "api_delete",
        [
            "Delete {resource}",
            "{resource} 지우기",
            "{resource} を削除",
            "删除 {resource}",
            "刪除 {resource}",
            "Eliminar {resource}",
            "Supprimer {resource}",
            "{resource} löschen",
            "Excluir {resource}",
            "Xóa {resource}",
            "Hapus {resource}",
            "Удалить {resource}",
        ],
    ),
    (
        "api_search",
        [
            "Search {resource}",
            "{resource} 검색",
            "{resource} を検索",
            "搜索 {resource}",
            "搜尋 {resource}",
            "Buscar {resource}",
            "Rechercher {resource}",
            "{resource} durchsuchen",
            "Pesquisar {resource}",
            "Tìm kiếm {resource}",
            "Cari {resource}",
            "Поиск {resource}",
        ],
    ),
    (
        "api_send",
        [
            "Send to {resource}",
            "{resource} 보내기",
            "{resource} に送信",
            "发送到 {resource}",
            "傳送到 {resource}",
            "Enviar a {resource}",
            "Envoyer à {resource}",
            "An {resource} senden",
            "Enviar para {resource}",
            "Gửi tới {resource}",
            "Kirim ke {resource}",
            "Отправить в {resource}",
        ],
    ),
    (
        "graphql_query",
        [
            "Query {name}",
            "{name} 조회",
            "{name} を照会",
            "查询 {name}",
            "查詢 {name}",
            "Consultar {name}",
            "Interroger {name}",
            "{name} abfragen",
            "Consultar {name}",
            "Truy vấn {name}",
            "Kueri {name}",
            "Запрос {name}",
        ],
    ),
    (
        "graphql_mutation",
        [
            "Run {name} (changes the site)",
            "{name} 실행 (변경)",
            "{name} を実行 (サイトを変更)",
            "运行 {name}（更改站点）",
            "執行 {name}（變更網站）",
            "Ejecutar {name} (cambia el sitio)",
            "Exécuter {name} (modifie le site)",
            "{name} ausführen (ändert die Site)",
            "Executar {name} (altera o site)",
            "Chạy {name} (thay đổi trang web)",
            "Jalankan {name} (mengubah situs)",
            "Выполнить {name} (изменяет сайт)",
        ],
    ),
];

/// The Korean word for a kind of record, when there is a common one; the
/// English words otherwise.
fn korean_what(what: &str) -> &str {
    match what {
        "news articles" => "뉴스 기사",
        "articles" | "blog postings" => "기사",
        "posts" => "글",
        "products" => "상품",
        "questions" => "질문",
        "projects" => "프로젝트",
        "events" => "행사",
        "job postings" | "jobs" => "채용 공고",
        "recipes" => "레시피",
        "videos" | "video objects" => "동영상",
        "books" => "책",
        "courses" => "강의",
        "people" | "persons" => "사람",
        "places" => "장소",
        "comments" => "댓글",
        "replies" => "답글",
        "reviews" => "후기",
        "content" => "콘텐츠",
        "items" | "entries" => "항목",
        "stories" | "submissions" => "글",
        "answers" => "답변",
        "lessons" => "강의",
        "releases" => "릴리스",
        "issues" => "이슈",
        "bugs" => "버그",
        "topics" => "주제",
        "threads" => "스레드",
        "discussions" => "토론",
        "notices" => "공지",
        "packages" => "패키지",
        "files" => "파일",
        "versions" => "버전",
        "commits" => "커밋",
        "tags" => "태그",
        "repositories" => "저장소",
        "papers" => "논문",
        "results" => "결과",
        "messages" => "메시지",
        "documents" => "문서",
        "photos" => "사진",
        "images" => "이미지",
        "offers" | "deals" => "할인",
        "listings" => "매물",
        "apps" => "앱",
        "episodes" => "에피소드",
        "tracks" => "곡",
        "albums" => "앨범",
        "movies" => "영화",
        "games" => "게임",
        "talks" => "발표",
        other => other,
    }
}

/// `exploreProjects` and `explore-projects` both read as `explore projects`.
pub(crate) fn words(name: &str) -> String {
    let mut out = String::new();
    let mut previous_lower = false;
    for c in name.chars() {
        if c.is_ascii_uppercase() && previous_lower {
            out.push(' ');
        }
        if c == '-' || c == '_' {
            out.push(' ');
            previous_lower = false;
        } else {
            out.push(c.to_ascii_lowercase());
            previous_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptions_read_as_a_person_would_say_them() {
        assert_eq!(
            describe(&Label::List { title: "Notices" }, "en"),
            "List what Notices shows"
        );
        assert_eq!(
            describe(
                &Label::List {
                    title: "공지사항"
                },
                "ko"
            ),
            "공지사항 목록 읽기"
        );
        assert_eq!(
            describe(&Label::Search { title: "" }, "ko"),
            "페이지에서 검색"
        );
        assert_eq!(
            describe(
                &Label::Api {
                    verb: "create",
                    resource: "notice"
                },
                "en"
            ),
            "Create notice"
        );
        assert_eq!(
            describe(
                &Label::Graphql {
                    name: "exploreProjects",
                    mutation: false
                },
                "ko"
            ),
            "explore projects 조회"
        );
        assert_eq!(
            describe(
                &Label::Graphql {
                    name: "create-issue",
                    mutation: true
                },
                "en"
            ),
            "Run create issue (changes the site)"
        );
        assert_eq!(
            describe(
                &Label::Records {
                    what: "news articles",
                    title: "BBC News"
                },
                "en"
            ),
            "List the news articles on BBC News"
        );
        assert_eq!(
            describe(
                &Label::Records {
                    what: "products",
                    title: "Shop"
                },
                "ko"
            ),
            "Shop의 상품 목록 읽기"
        );
        assert_eq!(language("", "ko-KR"), "ko");
        assert_eq!(language("en", "ko"), "en");
        assert_eq!(language("", ""), "en");
        assert_eq!(language("ja_JP.UTF-8", "en"), "ja");
        assert_eq!(language("C", "zh-TW"), "zh-Hant");
        assert_eq!(language("sv_SE", ""), "en");
        assert_eq!(
            describe(
                &Label::List {
                    title: "Hacker News"
                },
                "ja"
            ),
            "Hacker News の一覧を読む"
        );
        assert_eq!(
            describe(
                &Label::Api {
                    verb: "delete",
                    resource: "notice_board"
                },
                "de"
            ),
            "notice board löschen"
        );
        assert_eq!(
            describe(&Label::Feed { title: "" }, "ru"),
            "Прочитать ленту сайта"
        );
        assert_eq!(
            describe(
                &Label::Page {
                    title: &"x".repeat(200)
                },
                "en"
            )
            .chars()
            .count(),
            80
        );
    }

    fn fields(text: &str) -> std::collections::BTreeSet<&str> {
        text.match_indices('{')
            .filter_map(|(i, _)| text[i + 1..].split('}').next())
            .collect()
    }

    #[test]
    fn every_phrase_exists_in_every_language_with_the_same_fields() {
        let mut names = std::collections::BTreeSet::new();
        for (name, row) in TEMPLATES {
            assert!(names.insert(*name), "{name} is in the table twice");
            let wanted = fields(row[0]);
            for (tag, text) in LANGUAGES.iter().zip(row) {
                assert!(!text.trim().is_empty(), "{name}: {tag} is empty");
                assert_eq!(fields(text), wanted, "{name}: {tag} fills in other fields");
            }
        }
    }

    #[test]
    fn every_label_reads_in_every_language() {
        let labels = [
            Label::Page { title: "Board" },
            Label::List { title: "Board" },
            Label::Embedded { title: "Board" },
            Label::Records {
                what: "posts",
                title: "Board",
            },
            Label::One {
                what: "posts",
                title: "Board",
            },
            Label::Search { title: "" },
            Label::Submit { title: "Board" },
            Label::Upload { title: "Board" },
            Label::Attachment { title: "Board" },
            Label::Feed { title: "Board" },
            Label::Feed { title: "" },
            Label::Api {
                verb: "patch",
                resource: "notice",
            },
            Label::Graphql {
                name: "createIssue",
                mutation: true,
            },
        ];
        for tag in LANGUAGES {
            for label in &labels {
                let text = describe(label, tag);
                assert!(!text.is_empty() && !text.contains('{'), "{tag}: {text}");
            }
            // The site's own words stay as the site wrote them.
            assert!(describe(&labels[1], tag).contains("Board"), "{tag}");
            assert!(describe(&labels[12], tag).contains("create issue"), "{tag}");
        }
        assert_eq!(describe(&labels[0], "tlh"), "Read Board");
    }
}
