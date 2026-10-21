//! What `itda setup` says, in the person's language. Keys are what the
//! code asks for; English is the fallback for every key.
use std::sync::OnceLock;

/// `ko` when `ITDA_LANG` or `LANG` says so, else `en`.
pub fn lang() -> &'static str {
    static LANG: OnceLock<&'static str> = OnceLock::new();
    LANG.get_or_init(|| {
        let korean = crate::lang().to_ascii_lowercase().starts_with("ko");
        if korean { "ko" } else { "en" }
    })
}

pub fn t(key: &str) -> &'static str {
    let table: &[(&str, &str, &str)] = &[
        ("looking", "Looking at {host}...", "{host} 살펴보는 중..."),
        (
            "fetched_page",
            "The page opens without signing in.",
            "이 페이지는 로그인 없이 열립니다.",
        ),
        ("found", "Found: {what}.", "찾은 것: {what}."),
        ("found_list", "a list of {n} records", "목록 {n}건"),
        ("found_forms", "{n} forms", "폼 {n}개"),
        ("found_feed", "a feed", "피드"),
        ("found_data", "data the page embeds", "페이지에 담긴 데이터"),
        ("found_title", "only a title", "제목만"),
        (
            "needs_record",
            "The page could not be read as itself ({why}). itda can open your browser and watch while you use the site instead.",
            "itda 이름으로는 이 페이지를 읽지 못했습니다({why}). 대신 브라우저를 열어, 사이트를 쓰시는 동안 지켜볼 수 있습니다.",
        ),
        ("why_blocked", "it refused", "거부됨"),
        (
            "why_scripted",
            "it is drawn by scripts",
            "스크립트로 그려지는 페이지",
        ),
        ("why_signin", "it needs a sign-in", "로그인 필요"),
        (
            "ask_record",
            "Open the browser and record? [Y/n] ",
            "브라우저를 열어 녹화할까요? [Y/n] ",
        ),
        (
            "ask_record_anyway",
            "Does the site need a sign-in to be useful? Record in the browser instead of fetching the page. [y/N] ",
            "제대로 쓰려면 로그인이 필요한 사이트인가요? 페이지를 가져오는 대신 브라우저로 녹화합니다. [y/N] ",
        ),
        (
            "no_record_allowed",
            "The page could not be read and --no-record forbids the browser.",
            "페이지를 읽지 못했고 --no-record 때문에 브라우저를 열 수 없습니다.",
        ),
        (
            "no_terminal",
            "This needs a terminal to ask you questions; pass --no-record or --record, --client and --yes to run without one.",
            "질문을 하려면 터미널이 필요합니다. 터미널 없이 돌리려면 --no-record 또는 --record, --client, --yes를 주세요.",
        ),
        ("step", "{n}/5", "{n}/5"),
        (
            "actions_heading",
            "What agents will be able to do:",
            "에이전트가 할 수 있게 되는 일:",
        ),
        ("read", "read  ", "읽기  "),
        ("change", "change", "변경  "),
        ("inputs", "inputs: {list}", "입력: {list}"),
        (
            "change_note",
            "(waits for your approval each time)",
            "(실행 전마다 내 승인 필요)",
        ),
        (
            "exists",
            "'{site}' is already set up. Replace it? [y/N] ",
            "'{site}'는 이미 설정돼 있습니다. 바꿀까요? [y/N] ",
        ),
        (
            "exists_kept",
            "Kept the existing '{site}'. Run again with --yes, or a different --name, to replace it.",
            "기존 '{site}'를 그대로 두었습니다. 바꾸려면 --yes 또는 다른 --name으로 다시 실행하세요.",
        ),
        (
            "trying",
            "Trying every read once...",
            "읽기를 하나씩 실제로 불러보는 중...",
        ),
        ("ok_records", "{n} records", "{n}건"),
        ("ok", "ok", "성공"),
        (
            "skipped_needs",
            "not tried: needs {list}",
            "입력이 필요해 건너뜀: {list}",
        ),
        (
            "skipped_change",
            "not tried: changes the site",
            "변경이라 건너뜀",
        ),
        ("removed", "removed: {why}", "뺐음: {why}"),
        (
            "why_mismatch",
            "the answer was not what the page showed",
            "응답이 페이지와 달랐음",
        ),
        ("why_auth", "needs a sign-in", "로그인 필요"),
        ("saved", "Saved as '{site}'.", "'{site}'로 저장했습니다."),
        (
            "why_not_found",
            "the address no longer exists (404)",
            "주소가 없음 (404)",
        ),
        (
            "later",
            "Connect an agent any time with: itda mcp install claude-desktop (or cursor, vscode, codex, claude-code)",
            "에이전트 연결은 언제든: itda mcp install claude-desktop (또는 cursor, vscode, codex, claude-code)",
        ),
        (
            "ask_client",
            "Connect an agent? [1] Claude Desktop  [2] Claude Code  [3] Cursor  [4] VS Code  [5] Codex  [0] later: ",
            "에이전트에 연결할까요? [1] Claude Desktop  [2] Claude Code  [3] Cursor  [4] VS Code  [5] Codex  [0] 나중에: ",
        ),
        (
            "done",
            "Done. Ask your agent for something on '{site}', for example \"{example}\".",
            "끝났습니다. 에이전트에게 '{site}'에 대해 물어보세요. 예: \"{example}\".",
        ),
        ("example_read", "use {site} to {desc}", "{site}에서 {desc}"),
        (
            "approve_note",
            "A change runs only after you allow it here: itda approve <id>.",
            "변경은 여기서 `itda approve <id>`로 허락해야 실행됩니다.",
        ),
        (
            "nothing_works",
            "No read worked, so nothing was kept. Try `itda record {url}` and use the site while it watches.",
            "동작하는 읽기가 없어 아무것도 남기지 않았습니다. `itda record {url}`로 사이트를 쓰는 모습을 녹화해 보세요.",
        ),
    ];
    let lang = lang();
    table
        .iter()
        .find(|(k, _, _)| *k == key)
        .map(|(_, en, ko)| if lang == "ko" { *ko } else { *en })
        .unwrap_or("?")
}

/// `t(key)` with `{name}` fields filled in.
pub fn tf(key: &str, fields: &[(&str, &str)]) -> String {
    let mut out = t(key).to_owned();
    for (name, value) in fields {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}
