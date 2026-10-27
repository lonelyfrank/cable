"""Client HTTPS verso altri dispositivi LocalSend, con verifica dell'impronta del certificato."""
import hashlib
import http.client
import json
import secrets
import ssl


class IdentityMismatch(ssl.SSLError):
    """Il certificato presentato non corrisponde all'impronta attesa."""


def open_connection(host, port, identity, timeout, expected_fp=None):
    """Apre una connessione TLS e controlla l'impronta *effettiva* del certificato.

    Restituisce (HTTPSConnection già connessa, impronta effettiva).
    Non esiste alcun ripiego su HTTP.
    """
    conn = http.client.HTTPSConnection(host, port, timeout=timeout, context=identity.client_context())
    try:
        conn.connect()
        actual = hashlib.sha256(conn.sock.getpeercert(binary_form=True)).hexdigest().upper()
        if expected_fp and not secrets.compare_digest(actual, expected_fp.upper()):
            raise IdentityMismatch("L'impronta del certificato non corrisponde a quella annunciata")
    except BaseException:
        conn.close()
        raise
    return conn, actual


def json_request(host, port, method, path, identity, body=None, timeout=10, expected_fp=None):
    """Restituisce (stato, json|None, impronta_effettiva)."""
    conn, actual = open_connection(host, port, identity, timeout, expected_fp)
    try:
        data = json.dumps(body).encode() if body is not None else None
        headers = {"Content-Type": "application/json"} if data else {}
        conn.request(method, path, body=data, headers=headers)
        r = conn.getresponse()
        raw = r.read(8 * 1024 * 1024)
        try:
            parsed = json.loads(raw) if raw else None
        except ValueError:
            parsed = None
        return r.status, parsed, actual
    finally:
        conn.close()
