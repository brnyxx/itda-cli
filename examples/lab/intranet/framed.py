"""A notice board whose real content lives inside an iframe.

The home page is a shell: a heading and an `<iframe src="/frame/board">` on the
same origin. The frame holds a sign-in form and, once signed in, a second
`<iframe src="/frame/list">` with the notices table, paging and a post form.
Everything the intranet does, one and two levels down in frames instead of on
the page itself. This is the shape a plain fetch of the home page cannot see
into: it finds the shell and the iframe, never the board.

Reuses the intranet's board, sign-in and helpers. Standard library only.
`python framed.py [port]` listens on 0.0.0.0:8086.
"""

import html
import logging
import secrets
import sys
from http import HTTPStatus
from http.server import ThreadingHTTPServer
from urllib.parse import parse_qs, quote, urlsplit

import server
from server import PASSWORD, USER, Board

PAGE_SIZE = server.PAGE_SIZE
SESSION_COOKIE = "FRAMED_SESSION"
CSRF_COOKIE = "FRAMED_CSRF"

logger = logging.getLogger("framed")
BOARD = Board()


def table(rows):
    lines = [
        '<table id="notices"><thead><tr><th>번호</th><th>제목</th>'
        "<th>작성자</th><th>날짜</th></tr></thead><tbody>"
    ]
    for n in rows:
        lines.append(
            f'<tr class="notice"><td class="id">{n["id"]}</td>'
            f'<td class="title"><a href="/frame/notices/{n["id"]}">{html.escape(n["title"])}</a></td>'
            f'<td class="author">{html.escape(n["author"])}</td>'
            f'<td class="date">{n["date"]}</td></tr>'
        )
    lines.append("</tbody></table>")
    return "\n".join(lines)


def write_form(token):
    return f"""<h2>공지 작성</h2>
<form action="/frame/notices" method="post">
<input type="hidden" name="_csrf" value="{token}">
<label>제목 <input name="title" required></label>
<label>내용 <textarea name="body"></textarea></label>
<button type="submit">등록</button>
</form>"""


class Handler(server.Handler):
    server_version = "framed/0.1"
    session_cookie = SESSION_COOKIE
    csrf_cookie = CSRF_COOKIE

    def log_message(self, fmt, *args):
        logger.info("%s %s", self.address_string(), fmt % args)

    def session(self):
        sid = self.cookies().get(SESSION_COOKIE)
        return BOARD.sessions.get(sid) if sid else None

    def do_GET(self):
        parts = urlsplit(self.path)
        query = {k: v[0] for k, v in parse_qs(parts.query).items()}
        path = parts.path
        if path == "/health":
            self.send_page(HTTPStatus.OK, "ok")
        elif path == "/":
            self.shell()
        elif path == "/frame/board":
            self.frame_board()
        elif path == "/frame/list":
            self.frame_list(query)
        elif path == "/frame/login":
            self.login_form(query.get("next", "/frame/board"), "")
        elif path.startswith("/frame/notices/") and path[15:].isdigit():
            self.notice(int(path[15:]))
        else:
            self.send_page(HTTPStatus.NOT_FOUND, server.page("없는 페이지", "<p>없는 페이지입니다.</p>"))

    def do_POST(self):
        path = urlsplit(self.path).path
        if path == "/frame/login":
            self.login()
        elif path == "/frame/notices":
            self.post_notice()
        else:
            self.send_page(HTTPStatus.NOT_FOUND, server.page("없는 페이지", "<p>없는 페이지입니다.</p>"))

    def shell(self):
        body = '<iframe src="/frame/board" title="공지 게시판" width="100%" height="600"></iframe>'
        self.send_page(HTTPStatus.OK, server.page("사내 인트라넷", body))

    def require_frame_session(self, next_path):
        session = self.session()
        if session is None:
            self.redirect(f"/frame/login?next={quote(next_path)}")
        return session

    def frame_board(self):
        session = self.require_frame_session("/frame/board")
        if session is None:
            return
        body = '<iframe src="/frame/list" title="공지 목록" width="100%" height="520"></iframe>'
        self.send_page(HTTPStatus.OK, server.page("공지 게시판", body))

    def frame_list(self, query):
        session = self.require_frame_session("/frame/list")
        if session is None:
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
            f'<a href="/frame/list?page={n}">{n}</a>' if n != number else f"<strong>{n}</strong>"
            for n in range(1, pages + 1)
        )
        body = f"""{table(chunk)}
<p class="paging">{links}</p>
{write_form(session["csrf"])}"""
        self.send_page(HTTPStatus.OK, server.page("공지 목록", body))

    def notice(self, number):
        if self.require_frame_session(f"/frame/notices/{number}") is None:
            return
        with BOARD.lock:
            found = next((n for n in BOARD.notices if n["id"] == number), None)
        if found is None:
            self.send_page(HTTPStatus.NOT_FOUND, server.page("없는 공지", "<p>없는 공지입니다.</p>"))
            return
        body = (
            f'<p class="meta">{html.escape(found["author"])} · {found["date"]}</p>'
            f'<div class="body">{html.escape(found["body"])}</div>'
        )
        self.send_page(HTTPStatus.OK, server.page(found["title"], body))

    def login_form(self, next_path, error, status=HTTPStatus.OK):
        token = self.cookies().get(CSRF_COOKIE)
        headers = []
        if not token or token not in BOARD.csrf:
            token = secrets.token_hex(16)
            BOARD.csrf.add(token)
            headers.append(("Set-Cookie", f"{CSRF_COOKIE}={token}; Path=/; HttpOnly"))
        message = f'<p class="error">{html.escape(error)}</p>' if error else ""
        body = f"""{message}
<form action="/frame/login" method="post">
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
        next_path = form.get("next", "/frame/board")
        if not next_path.startswith("/") or next_path.startswith("//"):
            next_path = "/frame/board"
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
            self.redirect("/frame/login?next=/frame/board")
            return
        form = self.form_body()
        if form.get("_csrf") != session["csrf"]:
            self.send_page(HTTPStatus.FORBIDDEN, server.page("거부됨", "<p>토큰이 맞지 않아 등록하지 않았습니다.</p>"))
            return
        title = form.get("title", "").strip()
        if not title:
            self.send_page(HTTPStatus.BAD_REQUEST, server.page("거부됨", "<p>제목이 없습니다.</p>"))
            return
        notice = BOARD.add(title, form.get("body", ""), session["user"])
        self.redirect(f"/frame/notices/{notice['id']}")


def main():
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8086
    httpd = ThreadingHTTPServer(("0.0.0.0", port), Handler)
    logger.info("framed listening on %s", port)
    httpd.serve_forever()


if __name__ == "__main__":
    main()
