"""A tiny legacy intranet: a notice board behind a sign-in form.

Server-rendered HTML only, the way many older internal systems are built: a
login form with a CSRF token, a session cookie, a notices table with paging,
a search form, and a post form that only takes the token from its own page.
Everything lives in memory; restarting the server starts over.

Standard library only. `python server.py [port]` listens on 0.0.0.0:8085.
"""

import html
import logging
import secrets
import sys
import threading
from http import HTTPStatus
from http.cookies import SimpleCookie
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, quote, urlsplit

USER = "lab"
PASSWORD = "labpassword"
PAGE_SIZE = 10
SESSION_COOKIE = "INTRANET_SESSION"
CSRF_COOKIE = "INTRANET_CSRF"

logger = logging.getLogger("intranet")

AUTHORS = ["운영팀", "보안팀", "IT지원", "총무팀", "인사팀"]
TITLES = [
    "10월 정기 점검 안내",
    "보안 교육 이수 요청",
    "사내 메신저 업데이트",
    "주차장 공사 일정",
    "연말 휴가 신청 마감",
]


class Board:
    def __init__(self):
        self.lock = threading.Lock()
        self.notices = []
        self.sessions = {}
        self.csrf = set()
        for n in range(1, 26):
            self.notices.append(
                {
                    "id": n,
                    "title": f"{TITLES[(n - 1) % len(TITLES)]} #{n}",
                    "body": f"공지 {n}번의 본문입니다.",
                    "author": AUTHORS[(n - 1) % len(AUTHORS)],
                    "date": f"2026-10-{(n % 28) + 1:02d}",
                }
            )

    def add(self, title, body, author):
        with self.lock:
            notice = {
                "id": len(self.notices) + 1,
                "title": title,
                "body": body,
                "author": author,
                "date": "2026-10-22",
            }
            self.notices.append(notice)
            return notice

    def newest_first(self, query=""):
        with self.lock:
            rows = [n for n in self.notices if query.lower() in n["title"].lower()]
        return list(reversed(rows))


BOARD = Board()


def page(title, body):
    return f"""<!doctype html>
<html lang="ko"><head><meta charset="utf-8"><title>{html.escape(title)} - 사내 인트라넷</title></head>
<body><h1 id="board">{html.escape(title)}</h1>
{body}
</body></html>"""


def table(rows):
    lines = ["<table id=\"notices\"><thead><tr><th>번호</th><th>제목</th><th>작성자</th><th>날짜</th></tr></thead><tbody>"]
    for n in rows:
        lines.append(
            f"<tr class=\"notice\"><td class=\"id\">{n['id']}</td>"
            f"<td class=\"title\"><a href=\"/notices/{n['id']}\">{html.escape(n['title'])}</a></td>"
            f"<td class=\"author\">{html.escape(n['author'])}</td><td class=\"date\">{n['date']}</td></tr>"
        )
    lines.append("</tbody></table>")
    return "\n".join(lines)


def write_form(token):
    return f"""<h2>공지 작성</h2>
<form action="/notices" method="post">
<input type="hidden" name="_csrf" value="{token}">
<label>제목 <input name="title" required></label>
<label>내용 <textarea name="body"></textarea></label>
<button type="submit">등록</button>
</form>"""


class Handler(BaseHTTPRequestHandler):
    server_version = "intranet/0.1"

    def log_message(self, fmt, *args):
        logger.info("%s %s", self.address_string(), fmt % args)

    # ---- helpers -----------------------------------------------------------

    def cookies(self):
        jar = SimpleCookie()
        jar.load(self.headers.get("Cookie", ""))
        return {k: v.value for k, v in jar.items()}

    def session(self):
        sid = self.cookies().get(SESSION_COOKIE)
        return BOARD.sessions.get(sid) if sid else None

    def send_page(self, status, body, extra_headers=()):
        data = body.encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(data)))
        for key, value in extra_headers:
            self.send_header(key, value)
        self.end_headers()
        self.wfile.write(data)

    def redirect(self, location, extra_headers=()):
        self.send_response(HTTPStatus.SEE_OTHER)
        self.send_header("Location", location)
        self.send_header("Content-Length", "0")
        for key, value in extra_headers:
            self.send_header(key, value)
        self.end_headers()

    def form_body(self):
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length).decode("utf-8")
        return {k: v[0] for k, v in parse_qs(raw).items()}

    def require_session(self, path):
        session = self.session()
        if session is None:
            self.redirect(f"/login?next={quote(path)}")
        return session

    # ---- routes ------------------------------------------------------------

    def do_GET(self):
        parts = urlsplit(self.path)
        query = {k: v[0] for k, v in parse_qs(parts.query).items()}
        path = parts.path
        if path == "/health":
            self.send_page(HTTPStatus.OK, "ok")
        elif path == "/":
            self.send_page(HTTPStatus.OK, page("사내 인트라넷", '<ul><li><a href="/notices">공지사항</a></li></ul>'))
        elif path == "/login":
            self.login_form(query.get("next", "/notices"), "")
        elif path == "/notices":
            self.notices(query)
        elif path == "/search":
            self.search(query)
        elif path == "/notices/new":
            session = self.require_session(path)
            if session:
                self.send_page(HTTPStatus.OK, page("공지 작성", write_form(session["csrf"])))
        elif path.startswith("/notices/") and path[9:].isdigit():
            self.notice(int(path[9:]))
        else:
            self.send_page(HTTPStatus.NOT_FOUND, page("없는 페이지", "<p>없는 페이지입니다.</p>"))

    def do_POST(self):
        path = urlsplit(self.path).path
        if path == "/login":
            self.login()
        elif path == "/notices":
            self.post_notice()
        else:
            self.send_page(HTTPStatus.NOT_FOUND, page("없는 페이지", "<p>없는 페이지입니다.</p>"))

    def login_form(self, next_path, error, status=HTTPStatus.OK):
        token = self.cookies().get(CSRF_COOKIE)
        headers = []
        if not token or token not in BOARD.csrf:
            token = secrets.token_hex(16)
            BOARD.csrf.add(token)
            headers.append(("Set-Cookie", f"{CSRF_COOKIE}={token}; Path=/; HttpOnly"))
        message = f'<p class="error">{html.escape(error)}</p>' if error else ""
        body = f"""{message}
<form action="/login" method="post">
<input type="hidden" name="_csrf" value="{token}">
<input type="hidden" name="next" value="{html.escape(next_path)}">
<label>아이디 <input name="username" autocomplete="username"></label>
<label>비밀번호 <input type="password" name="password"></label>
<button type="submit">로그인</button>
</form>"""
        self.send_page(status, page("로그인", body), headers)

    def login(self):
        form = self.form_body()
        token = self.cookies().get(CSRF_COOKIE)
        next_path = form.get("next", "/notices")
        if not next_path.startswith("/") or next_path.startswith("//"):
            next_path = "/notices"
        if not token or form.get("_csrf") != token or token not in BOARD.csrf:
            self.login_form(next_path, "토큰이 맞지 않습니다. 다시 시도하세요.", HTTPStatus.FORBIDDEN)
            return
        if form.get("username") != USER or form.get("password") != PASSWORD:
            self.login_form(next_path, "아이디 또는 비밀번호가 틀립니다.")
            return
        sid = secrets.token_hex(16)
        BOARD.sessions[sid] = {"user": USER, "csrf": secrets.token_hex(16)}
        self.redirect(next_path, [("Set-Cookie", f"{SESSION_COOKIE}={sid}; Path=/; HttpOnly")])

    def notices(self, query):
        session = self.require_session("/notices")
        if not session:
            return
        try:
            number = max(1, int(query.get("page", "1")))
        except ValueError:
            number = 1
        rows = BOARD.newest_first()
        start = (number - 1) * PAGE_SIZE
        chunk = rows[start : start + PAGE_SIZE]
        pages = max(1, (len(rows) + PAGE_SIZE - 1) // PAGE_SIZE)
        links = " ".join(
            f'<a href="/notices?page={n}">{n}</a>' if n != number else f"<strong>{n}</strong>"
            for n in range(1, pages + 1)
        )
        body = f"""<form action="/search" method="get"><input name="q" placeholder="제목 검색" required><button type="submit">검색</button></form>
{table(chunk)}
<p class="paging">{links}</p>
{write_form(session["csrf"])}"""
        self.send_page(HTTPStatus.OK, page("공지사항", body))

    def search(self, query):
        if not self.require_session("/search"):
            return
        q = query.get("q", "")
        rows = BOARD.newest_first(q)
        body = f"<p>'{html.escape(q)}' 검색 결과 {len(rows)}건</p>\n{table(rows)}"
        self.send_page(HTTPStatus.OK, page("검색", body))

    def notice(self, number):
        if not self.require_session(f"/notices/{number}"):
            return
        with BOARD.lock:
            found = next((n for n in BOARD.notices if n["id"] == number), None)
        if found is None:
            self.send_page(HTTPStatus.NOT_FOUND, page("없는 공지", "<p>없는 공지입니다.</p>"))
            return
        body = f"<p class=\"meta\">{html.escape(found['author'])} · {found['date']}</p><div class=\"body\">{html.escape(found['body'])}</div>"
        self.send_page(HTTPStatus.OK, page(found["title"], body))

    def post_notice(self):
        session = self.session()
        if session is None:
            self.redirect("/login?next=/notices")
            return
        form = self.form_body()
        if form.get("_csrf") != session["csrf"]:
            self.send_page(HTTPStatus.FORBIDDEN, page("거부됨", "<p>토큰이 맞지 않아 등록하지 않았습니다.</p>"))
            return
        title = form.get("title", "").strip()
        if not title:
            self.send_page(HTTPStatus.BAD_REQUEST, page("거부됨", "<p>제목이 없습니다.</p>"))
            return
        notice = BOARD.add(title, form.get("body", ""), session["user"])
        self.redirect(f"/notices/{notice['id']}")


def main():
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8085
    server = ThreadingHTTPServer(("0.0.0.0", port), Handler)
    logger.info("intranet listening on %s", port)
    server.serve_forever()


if __name__ == "__main__":
    main()
