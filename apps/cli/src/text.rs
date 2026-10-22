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
            "more_pages",
            "There is another page: run the same command with --next {handle}",
            "다음 페이지가 있습니다. 같은 명령에 --next {handle}을 붙여 실행하세요",
        ),
        (
            "fetched_page",
            "The page opens without signing in.",
            "이 페이지는 로그인 없이 열립니다.",
        ),
        (
            "fetched_signed",
            "The page opens with the sign-in.",
            "로그인하니 이 페이지가 열립니다.",
        ),
        ("found", "Found: {what}.", "찾은 것: {what}."),
        (
            "recognised",
            "This looks like {software}. Using its API at {base}.",
            "{software}로 보입니다. {base}의 API를 씁니다.",
        ),
        (
            "bundled",
            "This is {title}. Using its {api}.",
            "{title}입니다. {api}를 씁니다.",
        ),
        (
            "template_unconfirmed",
            "Using the {software} template at {base} as asked; its API did not answer there yet.",
            "요청대로 {software} 템플릿을 {base}에 씁니다. 그 주소의 API는 아직 응답하지 않았습니다.",
        ),
        ("found_list", "a list of {n} records", "목록 {n}건"),
        ("found_form", "a form", "폼 1개"),
        ("found_forms", "{n} forms", "폼 {n}개"),
        ("found_feed", "a feed", "피드"),
        ("found_data", "data the page embeds", "페이지에 담긴 데이터"),
        ("found_main", "the page's own content", "본문"),
        (
            "found_description",
            "a link to its API description",
            "API 설명 링크",
        ),
        ("found_title", "only a title", "제목만"),
        (
            "followed_frame",
            "Followed 1 frame: {list}.",
            "프레임 1개를 따라가 읽었습니다: {list}.",
        ),
        (
            "followed_frames",
            "Followed {n} frames: {list}.",
            "프레임 {n}개를 따라가 읽었습니다: {list}.",
        ),
        (
            "asked_schema",
            "Asked the GraphQL API at {url} what it offers.",
            "{url}의 GraphQL API에 무엇을 제공하는지 물었습니다.",
        ),
        (
            "described",
            "Read its API description at {url}: {reads} reads, {changes} changes noted.",
            "{url}의 API 설명을 읽었습니다: 읽기 {reads}개, 메모로 남긴 변경 {changes}개.",
        ),
        (
            "needs_record",
            "itda could not read the page as itself ({why}). itda can open your browser and watch as you use the site instead.",
            "itda 이름으로는 이 페이지를 읽지 못했습니다({why}). 대신 브라우저를 열어 사이트를 쓰는 동안 지켜볼 수 있습니다.",
        ),
        ("why_blocked", "it refused", "거부됨"),
        (
            "blocked_cloudflare",
            "{host} is behind Cloudflare and refused itda.",
            "{host}는 Cloudflare 뒤에 있어 itda를 거부했습니다.",
        ),
        (
            "blocked_bot_manager",
            "{host} runs a bot manager that refused itda.",
            "{host}의 봇 차단이 itda를 거부했습니다.",
        ),
        (
            "blocked_captcha",
            "{host} asks for a captcha before it shows the page.",
            "{host}는 페이지를 보여주기 전에 캡차를 요구합니다.",
        ),
        (
            "blocked_other",
            "{host} refused itda (HTTP {status}).",
            "{host}가 itda를 거부했습니다 (HTTP {status}).",
        ),
        (
            "blocked_fix",
            "itda does not go around that; try the site's official API or ask the owner.",
            "itda는 이를 우회하지 않습니다. 사이트의 공식 API를 쓰거나 운영자에게 물어보세요.",
        ),
        (
            "rate_limited_wait",
            "{host} limits how often it is read and asked itda to wait {n} s.",
            "{host}는 읽는 횟수를 제한하며 itda에게 {n}초 기다리라고 했습니다.",
        ),
        (
            "rate_limited",
            "{host} limits how often it is read and refused itda for now.",
            "{host}는 읽는 횟수를 제한해 지금은 itda를 거부했습니다.",
        ),
        (
            "redirect_signin",
            "{host} sends itda to a sign-in page instead of showing {path}.",
            "{host}는 {path}를 보여주는 대신 itda를 로그인 페이지로 보냅니다.",
        ),
        (
            "redirect_signin_fix",
            "Pages behind a sign-in: record yourself using them with `itda record`, or `itda setup` and answer yes to recording.",
            "로그인이 필요한 페이지는 `itda record`로 직접 쓰는 모습을 녹화하거나 `itda setup`에서 녹화에 예라고 답하세요.",
        ),
        (
            "rate_limited_fix",
            "Try again after the wait, or use the site's official API.",
            "기다린 뒤 다시 시도하거나 사이트의 공식 API를 쓰세요.",
        ),
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
            "itda could not read the page; --no-record forbids the browser.",
            "페이지를 읽지 못했고 --no-record 때문에 브라우저를 열 수 없습니다.",
        ),
        (
            "watching",
            "Opening your browser on {host} to watch what the page loads by itself. itda does not click or type; the window closes on its own.",
            "{host} 페이지가 스스로 무엇을 불러오는지 보려고 브라우저를 엽니다. 아무것도 누르거나 입력하지 않으며 창은 저절로 닫힙니다.",
        ),
        (
            "watched",
            "Watched the page load: {n} calls, {k} reads drafted.",
            "페이지가 뜨는 동안 지켜봤습니다: 호출 {n}건, 읽기 초안 {k}개.",
        ),
        (
            "watched_nothing",
            "Watched the page load: {n} calls, nothing to draft.",
            "페이지가 뜨는 동안 지켜봤습니다: 호출 {n}건, 초안으로 만들 것 없음.",
        ),
        (
            "watch_only_stop",
            "Watching the page load was not enough ({why}), and --watch-only skips recording. Run again without it to record yourself using the site.",
            "페이지가 뜨는 동안 지켜본 것으로는 부족했고({why}) --watch-only는 녹화를 건너뜁니다. 직접 쓰는 모습을 녹화하려면 이 옵션 없이 다시 실행하세요.",
        ),
        (
            "no_terminal",
            "This needs a terminal to ask you questions; pass --no-record, --watch-only or --record, with --client and --yes, to run without one.",
            "질문을 하려면 터미널이 필요합니다. 터미널 없이 돌리려면 --no-record, --watch-only, --record 중 하나와 --client, --yes를 주세요.",
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
        ("required", "required", "필수"),
        ("default", "default {value}", "기본값 {value}"),
        ("example", "example: {command}", "예: {command}"),
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
            "withheld",
            "removed: {path}: the site does not give itda this page",
            "뺐음: {path}: 사이트가 itda에게 이 페이지를 주지 않습니다",
        ),
        (
            "skipped_refused",
            "not tried: the site refused an earlier read; itda check tries it",
            "앞선 읽기를 사이트가 거부해 건너뛰고 itda check 때 시도합니다",
        ),
        (
            "why_mismatch",
            "the answer was not what the page showed",
            "응답이 페이지와 달랐음",
        ),
        ("why_auth", "needs a sign-in", "로그인 필요"),
        (
            "why_wait",
            "it asked to wait {n} s between requests",
            "요청 사이에 {n}초 기다리라고 함",
        ),
        (
            "why_rate_limited",
            "it limits how often it is read",
            "읽는 횟수를 제한함",
        ),
        ("saved", "Saved as '{site}'.", "'{site}'로 저장했습니다."),
        (
            "why_not_found",
            "the address no longer exists (404)",
            "주소가 없음 (404)",
        ),
        (
            "later",
            "Connect an agent any time with: itda mcp install claude-desktop (or cursor, vscode, codex, claude-code)",
            "언제든 에이전트 연결: itda mcp install claude-desktop (또는 cursor, vscode, codex, claude-code)",
        ),
        (
            "ask_connect",
            "Connect {list}? [Y/n] ",
            "{list}에 연결할까요? [Y/n] ",
        ),
        ("and", " and ", "와(과) "),
        (
            "welcome",
            "itda hands the websites you use to your AI agent. Paste the address of a site you use and press Enter. No address in mind? Type github, npm, crates-io or pypi. Enter alone shows the command list.",
            "itda는 내가 쓰는 웹사이트를 AI 에이전트에게 넘겨줍니다. 내가 쓰는 사이트 주소를 붙여넣고 Enter를 누르세요. 떠오르는 주소가 없으면 github, npm, crates-io, pypi 중 하나를 치세요. 그냥 Enter는 명령 목록.",
        ),
        ("ask_url", "Address: ", "주소: "),
        (
            "ask_client",
            "Connect an agent? [1] Claude Desktop  [2] Claude Code  [3] Cursor  [4] VS Code  [5] Codex  [0] later: ",
            "에이전트에 연결할까요? [1] Claude Desktop  [2] Claude Code  [3] Cursor  [4] VS Code  [5] Codex  [0] 나중에: ",
        ),
        (
            "connecting",
            "Adding itda to {client}...",
            "{client}에 itda 추가 중...",
        ),
        (
            "added_client",
            "Added itda to {client} ({path}).",
            "{client}에 itda를 추가했습니다 ({path}).",
        ),
        (
            "already_client",
            "{client} already runs itda ({path}).",
            "{client}에는 itda가 이미 들어 있습니다 ({path}).",
        ),
        (
            "restart_hint",
            "Restart {client} to load it.",
            "{client}를 다시 시작하면 적용됩니다.",
        ),
        (
            "restart_ask",
            "{client} is open. Restart it now so itda shows up? [Y/n] ",
            "{client}가 열려 있습니다. itda가 보이도록 지금 다시 시작할까요? [Y/n] ",
        ),
        (
            "restarted",
            "{client} restarted; itda is in it.",
            "{client}를 다시 시작했습니다. itda가 들어 있습니다.",
        ),
        (
            "open_app",
            "Open {client} and itda will be there.",
            "{client}를 열면 itda가 보입니다.",
        ),
        ("running", "Running: {command}", "실행 중: {command}"),
        (
            "run_yourself",
            "{client} keeps its own configuration. Run:",
            "{client}는 설정을 직접 관리합니다. 이 명령을 실행하세요:",
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
            "refresh_from",
            "Drafting '{site}' again from {what}...",
            "'{site}'를 {what} 다시 초안 잡는 중...",
        ),
        ("from_pages", "its pages", "원래 페이지로"),
        ("from_recording", "its recording", "저장된 녹화로"),
        ("from_description", "its API description", "API 설명으로"),
        ("from_template", "its template", "템플릿으로"),
        (
            "refresh_before",
            "As installed: {verdict} ({hint}).",
            "지금 설치된 정의: {verdict} ({hint}).",
        ),
        (
            "refresh_same",
            "The sources draft the same actions; '{site}' stays as it is.",
            "같은 출처에서 같은 액션이 나옵니다. '{site}'는 그대로 둡니다.",
        ),
        ("refresh_heading", "What would change:", "바뀌는 것:"),
        ("refresh_added", "added: {desc}", "추가: {desc}"),
        ("refresh_removed", "removed: {why}", "삭제: {why}"),
        ("refresh_changed", "changed: {fields}", "변경: {fields}"),
        (
            "refresh_edited",
            "kept as you edited it",
            "직접 고친 그대로 유지",
        ),
        (
            "ask_refresh",
            "Write these changes to '{site}'? [y/N] ",
            "이 변경을 '{site}'에 쓸까요? [y/N] ",
        ),
        (
            "refresh_kept",
            "Kept '{site}' as it is. Run again with --yes to write the changes.",
            "'{site}'를 그대로 두었습니다. 쓰려면 --yes로 다시 실행하세요.",
        ),
        (
            "refresh_written",
            "Updated '{site}'. Checking it...",
            "'{site}'를 갱신했습니다. 확인하는 중...",
        ),
        (
            "ask_sign_in",
            "Sign in to {host} here, with your user name and password? itda fills in the site's own sign-in form, as itself, and keeps the session sealed under the data directory. [Y/n] ",
            "{host}에 여기서 아이디와 비밀번호로 로그인할까요? itda가 사이트의 로그인 폼을 itda 이름으로 보내고 세션은 데이터 폴더에 봉인해 보관합니다. [Y/n] ",
        ),
        ("ask_user", "User name for {site}: ", "{site} 아이디: "),
        (
            "ask_password",
            "Password for {user} on {site} (sent once to the site's own sign-in form, never saved): ",
            "{site}의 {user} 비밀번호 (사이트 로그인 폼에 한 번만 보내고 저장하지 않습니다): ",
        ),
        ("no_user", "No user name was given", "아이디가 없습니다"),
        (
            "no_user_fix",
            "Pass --user <name>, and the password as the first line of stdin.",
            "--user <아이디>를 주고 비밀번호는 stdin 첫 줄로 넘기세요.",
        ),
        (
            "no_password",
            "No password was given",
            "비밀번호가 없습니다",
        ),
        (
            "no_password_fix",
            "Type it after the question, or pipe it on stdin, for example from your password manager.",
            "질문 뒤에 입력하거나 비밀번호 관리자 등에서 stdin으로 넘기세요.",
        ),
        (
            "signed_in",
            "{host} accepted the sign-in ({n} cookies).",
            "{host}가 로그인을 받아들였습니다 (쿠키 {n}개).",
        ),
        (
            "session_kept",
            "Kept the session as {site}/{account}, sealed under the data directory with a key in the system keyring. Agents only see the account id; when it expires, `itda login {site}` signs in again.",
            "세션을 {site}/{account}로 데이터 폴더에 봉인해 보관했습니다 (키는 시스템 키링에). 에이전트는 계정 id만 봅니다. 만료되면 `itda login {site}`로 다시 로그인하세요.",
        ),
        (
            "session_saved",
            "Signed in. itda sealed the session for {site}/{account} ({n} cookies) under the data directory with a key in the system keyring; agents only see the account id.",
            "로그인했습니다. {site}/{account} 세션(쿠키 {n}개)을 데이터 폴더에 봉인했고(키는 시스템 키링에), 에이전트는 계정 id만 봅니다.",
        ),
        (
            "cookie_missing",
            "{site} accepted the sign-in, but the {name} cookie its definition names did not appear",
            "{site}가 로그인을 받아들였지만 정의에 적힌 {name} 쿠키가 오지 않았습니다",
        ),
        (
            "cookie_missing_fix",
            "Check value_from in the definition's auth entry, or remove it.",
            "정의의 auth 항목 value_from을 확인하거나 지우세요.",
        ),
        (
            "query_missing",
            "{site} accepted the sign-in, but did not hand out the {name} query parameter its definition names",
            "{site} 로그인은 됐지만 정의에 적힌 {name} 쿼리 매개변수가 오지 않았습니다",
        ),
        (
            "query_missing_fix",
            "Check value_from in the definition's auth entry; the key must appear in the sign-in's redirect or on the links of the first page after it.",
            "정의의 auth 항목 value_from을 확인하세요. 키는 로그인 뒤 리다이렉트 주소나 첫 페이지의 링크에 있어야 합니다.",
        ),
        (
            "session_saved_query",
            "Signed in. itda sealed the {name} session value for {site}/{account} under the data directory with a key in the system keyring. It goes on the address of each request to the site; agents only see the account id.",
            "로그인했습니다. {site}/{account}의 {name} 세션 값을 시스템 키링의 키로 데이터 폴더에 봉인했습니다. 사이트 요청 주소마다 붙입니다. 에이전트는 계정 id만 봅니다.",
        ),
        (
            "nothing_works",
            "No read worked; itda kept nothing. Try `itda record {url}` and use the site while it watches.",
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

/// How people say the client's name, from the slug `itda mcp install` takes.
pub fn client_label(slug: &str) -> &str {
    match slug {
        "claude-desktop" => "Claude Desktop",
        "claude-code" => "Claude Code",
        "cursor" => "Cursor",
        "vscode" => "VS Code",
        "codex" => "Codex",
        other => other,
    }
}

/// What a site's API description added to a draft, in one line.
pub fn described(described: &itda_prep::Described) -> String {
    tf(
        "described",
        &[
            ("url", &described.url),
            ("reads", &described.reads.to_string()),
            ("changes", &described.changes.to_string()),
        ],
    )
}

/// The frames a page's content was read through, in one line; `None` when
/// none was followed.
pub fn followed_frames(followed: &[String]) -> Option<String> {
    let list = followed.join(", ");
    match followed.len() {
        0 => None,
        1 => Some(tf("followed_frame", &[("list", &list)])),
        n => Some(tf(
            "followed_frames",
            &[("n", &n.to_string()), ("list", &list)],
        )),
    }
}

/// `t(key)` with `{name}` fields filled in.
pub fn tf(key: &str, fields: &[(&str, &str)]) -> String {
    let mut out = t(key).to_owned();
    for (name, value) in fields {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}
