"""A notice board a script draws from an XHR: the React-portal shape, no React.

`/board` is an empty shell, a heading and `<script src="/app.js">`. The script
asks `GET /api/notices?page=N` for JSON and draws `table#notices` from it, the
way a single-page app fills a portal. Reading the list needs no account, so a
plain fetch of the shell sees nothing and only a browser that runs the script
loads the table. Posting needs an account: `POST /api/notices` takes the
session cookie and an `X-CSRF-Token` the shell carries in
`<meta name="csrf-token">` once signed in.

Reuses the intranet's board and helpers. Standard library only.
`python scripted.py [port]` listens on 0.0.0.0:8087.
"""

import html
import json
import logging
import secrets
import sys
from http import HTTPStatus
from http.server import ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit

import server
from server import PASSWORD, USER, Board

PAGE_SIZE = server.PAGE_SIZE
SESSION_COOKIE = "SCRIPTED_SESSION"
CSRF_COOKIE = "SCRIPTED_CSRF"

logger = logging.getLogger("scripted")
BOARD = Board()

APP_JS = """// Draws the notices table from the JSON API, the way a portal would.
async function load(page) {
  const res = await fetch('/api/notices?page=' + (page || 1), {
    headers: { 'Accept': 'application/json' },
  });
  if (!res.ok) return;
  const data = await res.json();
  const root = document.getElementById('app');
  const rows = data.items.map(function (n) {
    return '<tr class="notice"><td class="id">' + n.id + '</td>' +
      '<td class="title"><a href="/notices/' + n.id + '">' + n.title + '</a></td>' +
      '<td class="author">' + n.author + '</td>' +
      '<td class="date">' + n.date + '</td></tr>';
  }).join('');
  root.innerHTML =
    '<table id="notices"><thead><tr><th>번호</th><th>제목</th>' +
    '<th>작성자</th><th>날짜</th></tr></thead><tbody>' + rows + '</tbody></table>';
}
load(1);
"""


class Handler(server.Handler):
    server_version = "scripted/0.1"

    def log_message(self, fmt, *args):
        logger.info("%s %s", self.address_string(), fmt % args)

    def session(self):
        sid = self.cookies().get(SESSION_COOKIE)
        return BOARD.sessions.get(sid) if sid else None

    def send_json(self, status, payload, extra_headers=()):
        data = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(data)))
        for key, value in extra_headers:
            self.send_header(key, value)
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        parts = urlsplit(self.path)
        query = {k: v[0] for k, v in parse_qs(parts.query).items()}
        path = parts.path
        if path == "/health":
            self.send_page(HTTPStatus.OK, "ok")
        elif path in ("/", "/board"):
            self.shell()
        elif path == "/app.js":
            self.app_js()
        elif path == "/api/notices":
            self.api_notices(query)
        elif path == "/login":
            self.login_form(query.get("next", "/board"), "")
        elif path.startswith("/notices/") and path[9:].isdigit():
            self.notice(int(path[9:]))
        else:
            self.send_json(HTTPStatus.NOT_FOUND, {"error": "not found"})

    def do_POST(self):
        path = urlsplit(self.path).path
        if path == "/login":
            self.login()
        elif path == "/api/notices":
            self.post_notice()
        else:
            self.send_json(HTTPStatus.NOT_FOUND, {"error": "not found"})

    def shell(self):
        session = self.session()
        token = session["csrf"] if session else ""
        body = (
            f'<!doctype html>\n<html lang="ko"><head><meta charset="utf-8">'
            f'<meta name="csrf-token" content="{token}">'
            f"<title>공지사항 - 사내 포털</title></head>\n"
            f'<body><h1 id="board">공지사항</h1>\n'
            f'<div id="app"></div>\n<script src="/app.js"></script>\n</body></html>'
        )
        self.send_page(HTTPStatus.OK, body)

    def app_js(self):
        data = APP_JS.encode("utf-8")
        self.send_response(HTTPStatus.OK)
        self.send_header("Content-Type", "text/javascript; charset=utf-8")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def api_notices(self, query):
        try:
            number = max(1, int(query.get("page", "1")))
        except ValueError:
            number = 1
        rows = BOARD.newest_first()
        start = (number - 1) * PAGE_SIZE
        chunk = rows[start : start + PAGE_SIZE]
        pages = max(1, (len(rows) + PAGE_SIZE - 1) // PAGE_SIZE)
        items = [
            {
                "id": n["id"],
                "title": n["title"],
                "author": n["author"],
                "date": n["date"],
                "link": f"/notices/{n['id']}",
            }
            for n in chunk
        ]
        self.send_json(
            HTTPStatus.OK,
            {"items": items, "page": number, "pages": pages, "total": len(rows)},
        )

    def notice(self, number):
        with BOARD.lock:
            found = next((n for n in BOARD.notices if n["id"] == number), None)
        if found is None:
            self.send_json(HTTPStatus.NOT_FOUND, {"error": "not found"})
            return
        self.send_json(HTTPStatus.OK, found)

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
        self.send_page(status, server.page("로그인", body), headers)

    def login(self):
        form = self.form_body()
        token = self.cookies().get(CSRF_COOKIE)
        next_path = form.get("next", "/board")
        if not next_path.startswith("/") or next_path.startswith("//"):
            next_path = "/board"
        if not token or form.get("_csrf") != token or token not in BOARD.csrf:
            self.login_form(next_path, "토큰이 맞지 않습니다. 다시 시도하세요.", HTTPStatus.FORBIDDEN)
            return
        if form.get("username") != USER or form.get("password") != PASSWORD:
            self.login_form(next_path, "아이디 또는 비밀번호가 틀립니다.")
            return
        sid = secrets.token_hex(16)
        BOARD.sessions[sid] = {"user": USER, "csrf": secrets.token_hex(16)}
        self.redirect(next_path, [("Set-Cookie", f"{SESSION_COOKIE}={sid}; Path=/; HttpOnly")])

    def post_notice(self):
        session = self.session()
        if session is None:
            self.send_json(HTTPStatus.UNAUTHORIZED, {"error": "sign in first"})
            return
        if self.headers.get("X-CSRF-Token") != session["csrf"]:
            self.send_json(HTTPStatus.FORBIDDEN, {"error": "bad csrf token"})
            return
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length).decode("utf-8")
        try:
            payload = json.loads(raw) if raw else {}
        except json.JSONDecodeError:
            self.send_json(HTTPStatus.BAD_REQUEST, {"error": "not json"})
            return
        title = (payload.get("title") or "").strip()
        if not title:
            self.send_json(HTTPStatus.BAD_REQUEST, {"error": "title is empty"})
            return
        notice = BOARD.add(title, payload.get("body", ""), session["user"])
        self.send_json(HTTPStatus.CREATED, notice)


def main():
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8087
    httpd = ThreadingHTTPServer(("0.0.0.0", port), Handler)
    logger.info("scripted listening on %s", port)
    httpd.serve_forever()


if __name__ == "__main__":
    main()
