"""The scripted portal with a lossy answer: a post is kept, then the line drops.

Every `POST /api/notices` the session and token allow is saved to the board
and the connection is closed without a status line, the way a proxy or a
load balancer times out after the application already committed the write.
`GET /api/notices` lists what was kept. itda sends the change once, gets no
answer, and runs the read `confirm` names to see whether it landed.

Reuses scripted.py's handler, board, cookies and shell, so the session cookie
is `SCRIPTED_SESSION` here too. Standard library only.
`python lossy.py [port]` listens on 0.0.0.0:8089.
"""

import logging
import socket
import sys
from http.server import ThreadingHTTPServer

import scripted

logger = logging.getLogger("lossy")


class Handler(scripted.Handler):
    server_version = "lossy/0.1"

    def log_message(self, fmt, *args):
        logger.info("%s %s", self.address_string(), fmt % args)

    def send_json(self, status, payload, extra_headers=()):
        if self.command == "POST" and self.path == "/api/notices" and status == 201:
            self.drop()
            return
        super().send_json(status, payload, extra_headers)

    def drop(self):
        """Close the line without answering; the record is already on the board."""
        self.close_connection = True
        try:
            self.connection.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        self.connection.close()


def main():
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8089
    httpd = ThreadingHTTPServer(("0.0.0.0", port), Handler)
    logger.info("lossy listening on %s", port)
    httpd.serve_forever()


if __name__ == "__main__":
    main()
