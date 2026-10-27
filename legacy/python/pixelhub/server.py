"""Server HTTPS del protocollo LocalSend v2 (ricezione e registrazione dispositivi)."""
import http.server
import json
import select
import socket
import ssl
import threading
import urllib.parse

from .bus import log
from .protocol import API, ProtocolError
from .receiver import PeerGone, StreamError


class BodyStream:
    """Corpo dell'upload: legge a blocchi (anche chunked) e traduce gli errori di rete."""

    def __init__(self, handler):
        self.h = handler
        te = (handler.headers.get("Transfer-Encoding") or "").lower()
        self.chunked = "chunked" in te
        cl = handler.headers.get("Content-Length")
        if not self.chunked and cl is None:
            raise ProtocolError(411, "Content-Length mancante")
        try:
            self.length = None if self.chunked else int(cl)
        except ValueError:
            raise ProtocolError(400, "Content-Length non valido")
        if self.length is not None and self.length < 0:
            raise ProtocolError(400, "Content-Length non valido")

    def read(self, write):
        rf = self.h.rfile
        try:
            if self.chunked:
                while True:
                    line = rf.readline(65537)
                    if not line:
                        raise StreamError("connessione chiusa")
                    size = int(line.split(b";")[0].strip() or b"0", 16)
                    if size == 0:
                        while rf.readline(65537) not in (b"\r\n", b"\n", b""):
                            pass
                        return
                    left = size
                    while left:
                        c = rf.read(min(left, 1 << 20))
                        if not c:
                            raise StreamError("connessione chiusa")
                        write(c)
                        left -= len(c)
                    rf.readline()
            else:
                left = self.length
                while left > 0:
                    c = rf.read(min(left, 1 << 20))
                    if not c:
                        raise StreamError("connessione chiusa")
                    write(c)
                    left -= len(c)
        except (OSError, ValueError) as e:
            # gli errori di scrittura su disco hanno origine in write(): li lasciamo passare
            if isinstance(e, (ssl.SSLError, ConnectionError, socket.timeout, ValueError)):
                raise StreamError(str(e)) from e
            raise


class LocalSendServer(http.server.ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True

    def __init__(self, address, engine):
        self.engine = engine
        self.ctx = engine.identity.server_context()
        self._slots = threading.BoundedSemaphore(engine.receiver.limits.max_connections)
        super().__init__(address, Handler)

    def get_request(self):
        sock, addr = self.socket.accept()
        return self.ctx.wrap_socket(sock, server_side=True, do_handshake_on_connect=False), addr

    def process_request(self, request, client_address):
        if not self._slots.acquire(blocking=False):
            self.shutdown_request(request)
            return
        super().process_request(request, client_address)

    def process_request_thread(self, request, client_address):
        try:
            super().process_request_thread(request, client_address)
        finally:
            self._slots.release()

    def handle_error(self, request, client_address):
        log("errore connessione da", client_address)


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "PixelHub"

    def log_message(self, *a):
        log("LS", self.client_address[0], self.command, urllib.parse.urlsplit(self.path).path)

    @property
    def engine(self):
        return self.server.engine

    def setup(self):
        self.request.settimeout(10)
        self.request.do_handshake()
        self.request.settimeout(30)
        super().setup()

    # ------------------------------------------------------------ risposta
    def reply(self, status, obj=None):
        data = b"" if obj is None else json.dumps(obj).encode()
        self.send_response(status)
        if status >= 400:
            self.close_connection = True
            self.send_header("Connection", "close")
        if obj is not None:
            self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        if data:
            self.wfile.write(data)

    def json_body(self):
        try:
            n = int(self.headers.get("Content-Length") or 0)
        except ValueError:
            raise ProtocolError(400, "Content-Length non valido")
        limit = self.engine.receiver.limits.max_json_body
        if n <= 0 or n > limit:
            raise ProtocolError(400 if n <= 0 else 413, "corpo non valido")
        try:
            return json.loads(self.rfile.read(n))
        except (ValueError, OSError):
            raise ProtocolError(400, "JSON non valido")

    def peer_closed(self):
        """True se il mittente ha chiuso la connessione mentre attende la risposta."""
        try:
            r, _, _ = select.select([self.request], [], [], 0)
            if not r:
                return False
            return self.request.recv(1) == b""
        except (OSError, ValueError):
            return True

    # ------------------------------------------------------------ routing
    def do_GET(self):
        try:
            if urllib.parse.urlparse(self.path).path in (API + "/info",):
                return self.reply(200, self.engine.info(with_port=False))
            self.reply(404)
        except Exception as e:  # noqa: BLE001
            self._fail(e)

    def do_POST(self):
        u = urllib.parse.urlparse(self.path)
        q = dict(urllib.parse.parse_qsl(u.query))
        ip = self.client_address[0]
        eng = self.engine
        try:
            if u.path == API + "/register":
                eng.discovery.on_register(self.json_body(), ip)
                return self.reply(200, eng.info(with_port=False))
            if u.path == API + "/prepare-upload":
                status, obj = eng.receiver.prepare(ip, q.get("pin"), self.json_body(), self.peer_closed)
                return self.reply(status, obj)
            if u.path == API + "/upload":
                eng.receiver.upload(ip, q, BodyStream(self))
                return self.reply(200)
            if u.path == API + "/cancel":
                eng.receiver.cancel_from_sender(ip, q.get("sessionId", ""))
                return self.reply(200)
            self.reply(404)
        except Exception as e:  # noqa: BLE001
            self._fail(e)

    def _fail(self, e):
        if isinstance(e, ProtocolError):
            return self.reply(e.status, {"message": e.message} if e.message else None)
        if isinstance(e, (PeerGone, StreamError)):
            self.close_connection = True
            return
        log("errore interno", repr(e))
        self.close_connection = True
        try:
            self.reply(500)
        except OSError:
            pass
