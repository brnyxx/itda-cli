"""A notice board that carries its session in the URL, not a cookie.

Signing in answers with a redirect to `/notices?sid=<token>`. Every link and
form on the site carries that `sid`, and a request without a valid one is sent
to `/login`. There is no cookie at all. A post form sends `sid` in the query as
well. This is the shape that breaks a client which keeps sessions in cookies:
follow the redirect, drop the token, and the next request is a sign-in page.

Reuses the intranet's board and helpers. Standard library only.
`python urltoken.py [port]` listens on 0.0.0.0:8088.
"""

import html
import logging
import secrets
import sys
from http import HTTPStatus
from http.server import ThreadingHTTPServer
from urllib.parse import parse_qs, urlencode, urlsplit

import server
from server import PASSWORD, USER, Board

PAGE_SIZE = server.PAGE_SIZE

logger = logging.getLogger("urltoken")
BOARD = Board()


def table(rows, sid):
    q = urlencode({"sid": sid})
    lines = [
        '<table id="notices"><thead><tr><th>번호</th><th>제목</th>'
        "<th>작성자</th><th>날짜</th></tr></thead><tbody>"
    ]
    for n in rows:
        lines.append(
            f'<tr class="notice"><td class="id">{n["id"]}</td>'
            f'<td class="title"><a href="/notices/{n["id"]}?{q}">{html.escape(n["title"])}</a></td>'
            f'<td class="author">{html.escape(n["author"])}</td>'
            f'<td class="date">{n["date"]}</td></tr>'
        )
    lines.append("</tbody></table>")
    return "\n".join(lines)


def write_form(sid):
    q = urlencode({"sid": sid})
    return f"""<h2>공지 작성</h2>
<form action="/notices?{q}" method="post">
<label>제목 <input name="title" required></label>
<label>내용 <textarea name="body"></textarea></label>
<button type="submit">등록</button>
</form>"""


class Handler(server.Handler):
    server_version = "urltoken/0.1"

    def log_message(self, fmt, *args):
        logger.info("%s %s", self.address_string(), fmt % args)

    def valid_sid(self, query):
        sid = query.get("sid")
        return sid if sid and sid in BOARD.sessions else None

    def do_GET(self):
        parts = urlsplit(self.path)
        query = {k: v[0] for k, v in parse_qs(parts.query).items()}
        path = parts.path
        if path == "/health":
            self.send_page(HTTPStatus.OK, "ok")
        elif path == "/login":
            self.login_form("")
        elif path == "/notices":
            self.notices(query)
        elif path.startswith("/notices/") and path[9:].isdigit():
            self.notice(int(path[9:]), query)
        elif path == "/":
            self.redirect("/login")
        else:
            self.send_page(HTTPStatus.NOT_FOUND, server.page("없는 페이지", "<p>없는 페이지입니다.</p>"))

    def do_POST(self):
        parts = urlsplit(self.path)
        query = {k: v[0] for k, v in parse_qs(parts.query).items()}
        if parts.path == "/login":
            self.login()
        elif parts.path == "/notices":
            self.post_notice(query)
        else:
            self.send_page(HTTPStatus.NOT_FOUND, server.page("없는 페이지", "<p>없는 페이지입니다.</p>"))

    def login_form(self, error, status=HTTPStatus.OK):
        token = secrets.token_hex(16)
        BOARD.csrf.add(token)
        message = f'<p class="error">{html.escape(error)}</p>' if error else ""
        body = f"""{message}
<form action="/login" method="post">
<input type="hidden" name="_csrf" value="{token}">
<label>아이디 <input name="username" autocomplete="username"></label>
<label>비밀번호 <input type="password" name="password"></label>
<button type="submit">로그인</button>
</form>"""
        self.send_page(status, server.page("로그인", body))

    def login(self):
        form = self.form_body()
        token = form.get("_csrf")
        if not token or token not in BOARD.csrf:
            self.login_form("토큰이 맞지 않습니다. 다시 시도하세요.", HTTPStatus.FORBIDDEN)
            return
        if form.get("username") != USER or form.get("password") != PASSWORD:
            self.login_form("아이디 또는 비밀번호가 틀립니다.")
            return
        sid = secrets.token_hex(16)
        BOARD.sessions[sid] = {"user": USER}
        self.redirect(f"/notices?{urlencode({'sid': sid})}")

    def notices(self, query):
        sid = self.valid_sid(query)
        if not sid:
            self.redirect("/login")
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
            f'<a href="/notices?{urlencode({"page": n, "sid": sid})}">{n}</a>'
            if n != number
            else f"<strong>{n}</strong>"
            for n in range(1, pages + 1)
        )
        body = f"""{table(chunk, sid)}
<p class="paging">{links}</p>
{write_form(sid)}"""
        self.send_page(HTTPStatus.OK, server.page("공지사항", body))

    def notice(self, number, query):
        sid = self.valid_sid(query)
        if not sid:
            self.redirect("/login")
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

    def post_notice(self, query):
        sid = self.valid_sid(query)
        if not sid:
            self.redirect("/login")
            return
        form = self.form_body()
        title = form.get("title", "").strip()
        if not title:
            self.send_page(HTTPStatus.BAD_REQUEST, server.page("거부됨", "<p>제목이 없습니다.</p>"))
            return
        notice = BOARD.add(title, form.get("body", ""), BOARD.sessions[sid]["user"])
        self.redirect(f"/notices/{notice['id']}?{urlencode({'sid': sid})}")


def main():
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8088
    httpd = ThreadingHTTPServer(("0.0.0.0", port), Handler)
    logger.info("urltoken listening on %s", port)
    httpd.serve_forever()


if __name__ == "__main__":
    main()
