//! What a person can do with a site, in their language, made by rule from
//! the action descriptions: what can be read, what can be changed, a few
//! ways to ask an agent, and how one read's answer feeds another's input.
//! No model writes any of it.
use itda_types::lang::LANGUAGES;
use serde::{Deserialize, Serialize};

/// One read whose answer carries the input of another.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flow {
    /// The read that lists things.
    pub list: String,
    /// Where in its answer the value is: `/items/0/id`.
    pub pointer: String,
    /// The read that needs it.
    pub detail: String,
    pub input: String,
}

/// An action as the guide needs it.
pub struct Doing<'a> {
    pub name: &'a str,
    pub desc: &'a str,
    pub read: bool,
    /// The inputs it cannot run without.
    pub needs: &'a [String],
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Guide {
    /// One line: what can be read.
    pub reads: Option<String>,
    /// One line: what can be changed, each after approval.
    pub changes: Option<String>,
    /// One line per flow between reads.
    pub flows: Vec<String>,
}

fn template(key: &str, lang: &str) -> &'static str {
    let column = LANGUAGES.iter().position(|tag| *tag == lang).unwrap_or(0);
    TEMPLATES
        .iter()
        .find(|(name, _)| *name == key)
        .map_or("", |(_, row)| row[column])
}

fn fill(text: &str, fields: &[(&str, &str)]) -> String {
    let mut out = text.to_owned();
    for (name, value) in fields {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// A description where a sentence continues: `List what X shows` becomes
/// `list what X shows` in the languages that start a sentence with a
/// capital. Other scripts are left as they are.
fn lowered(desc: &str) -> String {
    let mut chars = desc.chars();
    match (chars.next(), chars.next()) {
        (Some(first), Some(second)) if first.is_uppercase() && second.is_lowercase() => {
            let mut out: String = first.to_lowercase().collect();
            out.push(second);
            out.push_str(chars.as_str());
            out
        }
        _ => desc.to_owned(),
    }
}

/// The lines of the guide, in `lang` (one of [`LANGUAGES`]; English for
/// anything else). Descriptions are used as the site's definition has them,
/// names where it has none.
pub fn guide(actions: &[Doing<'_>], flows: &[Flow], lang: &str) -> Guide {
    // An action without a description goes by its name.
    let join = |read: bool| -> Option<String> {
        let list: Vec<&str> = actions
            .iter()
            .filter(|a| a.read == read)
            .map(|a| match a.desc.trim() {
                "" => a.name,
                desc => desc,
            })
            .collect();
        (!list.is_empty()).then(|| list.join("; "))
    };
    Guide {
        reads: join(true).map(|list| fill(template("can_read", lang), &[("list", &list)])),
        changes: join(false).map(|list| fill(template("can_change", lang), &[("list", &list)])),
        flows: flows
            .iter()
            .map(|flow| {
                fill(
                    template("flow", lang),
                    &[
                        ("detail", &flow.detail),
                        ("input", &flow.input),
                        ("list", &flow.list),
                        ("pointer", &flow.pointer),
                    ],
                )
            })
            .collect(),
    }
}

/// Two to four things a person might ask an agent, in `lang`: the first
/// read that needs nothing, a read that needs an input, a change (which
/// they approve), and a check of the site called `title`. Each at most
/// once.
pub fn prompts(title: &str, actions: &[Doing<'_>], lang: &str) -> Vec<String> {
    let mut out = Vec::new();
    // A description that starts the sentence keeps its capital.
    let ask = |key: &str, action: &Doing<'_>, fields: &[(&str, &str)]| {
        let text = template(key, lang);
        let desc = if text.starts_with("{desc}") {
            action.desc.trim().to_owned()
        } else {
            lowered(action.desc.trim())
        };
        let mut all = vec![("desc", desc.as_str())];
        all.extend_from_slice(fields);
        fill(text, &all)
    };
    if let Some(read) = actions
        .iter()
        .find(|a| a.read && a.needs.is_empty() && !a.desc.trim().is_empty())
    {
        out.push(ask("prompt_read", read, &[]));
    }
    if let Some(read) = actions
        .iter()
        .find(|a| a.read && !a.needs.is_empty() && !a.desc.trim().is_empty())
    {
        out.push(ask("prompt_read_input", read, &[("input", &read.needs[0])]));
    }
    if let Some(change) = actions
        .iter()
        .find(|a| !a.read && !a.desc.trim().is_empty())
    {
        out.push(ask("prompt_change", change, &[]));
    }
    if out.len() < 4 && !title.trim().is_empty() {
        out.push(fill(
            template("prompt_check", lang),
            &[("title", title.trim())],
        ));
    }
    out
}

/// One row per phrase, one column per language in the order of
/// [`LANGUAGES`]. `{desc}` is an action's description as written, `{list}`
/// several of them, `{title}` the site's title; `{detail}`, `{input}`,
/// `{list}` and `{pointer}` in `flow` name actions and places. Every
/// language keeps the fields English has (a test checks that).
const TEMPLATES: &[(&str, [&str; 12])] = &[
    (
        "can_read",
        [
            "You can read: {list}",
            "읽을 수 있는 것: {list}",
            "読めるもの: {list}",
            "可以读取: {list}",
            "可以讀取: {list}",
            "Puedes leer: {list}",
            "Vous pouvez lire : {list}",
            "Sie können lesen: {list}",
            "Você pode ler: {list}",
            "Bạn có thể đọc: {list}",
            "Anda dapat membaca: {list}",
            "Можно читать: {list}",
        ],
    ),
    (
        "can_change",
        [
            "You can change, after approving each one: {list}",
            "바꿀 수 있는 것(하나씩 승인한 뒤): {list}",
            "変更できるもの（それぞれ承認した後）: {list}",
            "可以更改（每项需先经你批准）: {list}",
            "可以更改（每項需先經你核准）: {list}",
            "Puedes cambiar, aprobando cada uno: {list}",
            "Vous pouvez modifier, après avoir approuvé chaque action : {list}",
            "Sie können ändern, nach Freigabe jeder einzelnen Aktion: {list}",
            "Você pode alterar, aprovando cada uma: {list}",
            "Bạn có thể thay đổi, sau khi duyệt từng thao tác: {list}",
            "Anda dapat mengubah, setelah menyetujui masing-masing: {list}",
            "Можно изменять, подтверждая каждое действие: {list}",
        ],
    ),
    (
        "flow",
        [
            "{detail} needs {input}; {list} gives it at {pointer}",
            "{detail}에는 {input}이(가) 필요합니다. {list}의 {pointer}에서 얻을 수 있습니다",
            "{detail} には {input} が必要です。{list} の {pointer} から得られます",
            "{detail} 需要 {input}；可从 {list} 的 {pointer} 取得",
            "{detail} 需要 {input}；可從 {list} 的 {pointer} 取得",
            "{detail} necesita {input}; {list} lo da en {pointer}",
            "{detail} a besoin de {input} ; {list} le donne à {pointer}",
            "{detail} braucht {input}; {list} liefert es unter {pointer}",
            "{detail} precisa de {input}; {list} o fornece em {pointer}",
            "{detail} cần {input}; {list} cung cấp nó tại {pointer}",
            "{detail} membutuhkan {input}; {list} menyediakannya di {pointer}",
            "{detail} требует {input}; {list} даёт его в {pointer}",
        ],
    ),
    (
        "prompt_read",
        [
            "Can you {desc}?",
            "{desc} 해줘.",
            "「{desc}」をお願い。",
            "请{desc}。",
            "請{desc}。",
            "¿Puedes {desc}?",
            "Peux-tu {desc} ?",
            "Kannst du {desc}?",
            "Você pode {desc}?",
            "Bạn có thể {desc} không?",
            "Bisakah kamu {desc}?",
            "Пожалуйста, {desc}.",
        ],
    ),
    (
        "prompt_read_input",
        [
            "Can you {desc}? The {input} is ...",
            "{desc} 해줘. {input}은(는) ...이야.",
            "「{desc}」をお願い。{input} は ... です。",
            "请{desc}。{input} 是 ...。",
            "請{desc}。{input} 是 ...。",
            "¿Puedes {desc}? El {input} es ...",
            "Peux-tu {desc} ? Le {input} est ...",
            "Kannst du {desc}? Der Wert für {input} ist ...",
            "Você pode {desc}? O {input} é ...",
            "Bạn có thể {desc} không? {input} là ...",
            "Bisakah kamu {desc}? {input}-nya adalah ...",
            "Пожалуйста, {desc}. {input}: ...",
        ],
    ),
    (
        "prompt_change",
        [
            "Please {desc}. I will approve it in the terminal.",
            "{desc} 해줘. 터미널에서 내가 승인할게.",
            "「{desc}」をお願い。ターミナルで私が承認します。",
            "请{desc}。我会在终端里批准。",
            "請{desc}。我會在終端機裡核准。",
            "Por favor, {desc}. Lo apruebo en la terminal.",
            "S'il te plaît, {desc}. Je l'approuverai dans le terminal.",
            "Bitte {desc}. Ich gebe es im Terminal frei.",
            "Por favor, {desc}. Eu aprovo no terminal.",
            "Hãy {desc}. Tôi sẽ duyệt trong terminal.",
            "Tolong {desc}. Saya akan menyetujuinya di terminal.",
            "Пожалуйста, {desc}. Я подтвержу это в терминале.",
        ],
    ),
    (
        "prompt_check",
        [
            "Does {title} still work? Check it.",
            "{title} 아직 잘 되는지 확인해줘.",
            "{title} はまだ動く？確認して。",
            "{title} 还能用吗？检查一下。",
            "{title} 還能用嗎？檢查一下。",
            "¿{title} sigue funcionando? Compruébalo.",
            "{title} fonctionne-t-il toujours ? Vérifie.",
            "Funktioniert {title} noch? Prüf es.",
            "{title} ainda funciona? Verifique.",
            "{title} còn hoạt động không? Kiểm tra giúp tôi.",
            "Apakah {title} masih berfungsi? Periksa.",
            "{title} ещё работает? Проверь.",
        ],
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn actions() -> Vec<(String, String, bool, Vec<String>)> {
        vec![
            (
                "list-notices".into(),
                "List what Notices shows".into(),
                true,
                vec![],
            ),
            (
                "get-notice".into(),
                "Read one of the notices on Notices".into(),
                true,
                vec!["id".into()],
            ),
            (
                "post-notice".into(),
                "Send the form on Notices".into(),
                false,
                vec!["title".into()],
            ),
        ]
    }

    fn doings(rows: &[(String, String, bool, Vec<String>)]) -> Vec<Doing<'_>> {
        rows.iter()
            .map(|(name, desc, read, needs)| Doing {
                name,
                desc,
                read: *read,
                needs,
            })
            .collect()
    }

    #[test]
    fn the_guide_says_what_can_be_read_and_changed() {
        let rows = actions();
        let flows = [Flow {
            list: "list-notices".into(),
            pointer: "/items/0/id".into(),
            detail: "get-notice".into(),
            input: "id".into(),
        }];
        let en = guide(&doings(&rows), &flows, "en");
        assert_eq!(
            en.reads.as_deref(),
            Some("You can read: List what Notices shows; Read one of the notices on Notices")
        );
        assert_eq!(
            en.changes.as_deref(),
            Some("You can change, after approving each one: Send the form on Notices")
        );
        assert_eq!(
            en.flows,
            ["get-notice needs id; list-notices gives it at /items/0/id"]
        );
        let unnamed = [("delete".to_owned(), String::new(), false, Vec::new())];
        assert_eq!(
            guide(&doings(&unnamed), &[], "en").changes.as_deref(),
            Some("You can change, after approving each one: delete")
        );
        let ko = guide(&doings(&rows[..1]), &[], "ko");
        assert_eq!(
            ko.reads.as_deref(),
            Some("읽을 수 있는 것: List what Notices shows")
        );
        assert_eq!(ko.changes, None);
        assert!(ko.flows.is_empty());
    }

    #[test]
    fn prompts_are_two_to_four_and_read_as_a_person_would_ask() {
        let rows = actions();
        let en = prompts("Notices", &doings(&rows), "en");
        assert_eq!(
            en,
            [
                "Can you list what Notices shows?",
                "Can you read one of the notices on Notices? The id is ...",
                "Please send the form on Notices. I will approve it in the terminal.",
                "Does Notices still work? Check it.",
            ]
        );
        let ko = prompts("공지", &doings(&rows[..1]), "ko");
        assert_eq!(
            ko,
            [
                "List what Notices shows 해줘.",
                "공지 아직 잘 되는지 확인해줘."
            ]
        );
        assert_eq!(prompts("T", &[], "ja"), ["T はまだ動く？確認して。"]);
        assert!(prompts("", &[], "ja").is_empty());
        for lang in LANGUAGES {
            let all = prompts("T", &doings(&rows), lang);
            assert!((2..=4).contains(&all.len()), "{lang}: {all:?}");
        }
    }

    #[test]
    fn every_language_keeps_the_fields() {
        let fields = |text: &str| -> Vec<String> {
            let mut out: Vec<String> = text
                .match_indices('{')
                .filter_map(|(i, _)| text[i + 1..].split('}').next().map(str::to_owned))
                .collect();
            out.sort();
            out
        };
        for (key, row) in TEMPLATES {
            let wanted = fields(row[0]);
            for (i, text) in row.iter().enumerate() {
                assert_eq!(fields(text), wanted, "{key} in {}", LANGUAGES[i]);
                assert!(!text.trim().is_empty(), "{key} in {}", LANGUAGES[i]);
            }
        }
    }
}
