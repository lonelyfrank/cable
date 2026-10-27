"""Identità TLS persistente (certificato autofirmato + chiave) nei dati utente."""
import datetime
import hashlib
import os
import shutil
import ssl
import time
from pathlib import Path

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509.oid import NameOID


def fingerprint_der(der):
    return hashlib.sha256(der).hexdigest().upper()


def _write_private(path, data):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "wb") as f:
        f.write(data)
    os.chmod(path, 0o600)


def _check_pair(cert_path, key_path):
    """Restituisce l'impronta se certificato e chiave sono validi e corrispondono."""
    cert = x509.load_pem_x509_certificate(Path(cert_path).read_bytes())
    key = serialization.load_pem_private_key(Path(key_path).read_bytes(), None)
    pub = lambda k: k.public_bytes(serialization.Encoding.DER, serialization.PublicFormat.SubjectPublicKeyInfo)
    if pub(cert.public_key()) != pub(key.public_key()):
        raise ValueError("la chiave non corrisponde al certificato")
    return fingerprint_der(cert.public_bytes(serialization.Encoding.DER))


class Identity:
    """stato: 'ok' | 'invalida'. Un'identità illeggibile NON viene rigenerata da sola."""

    def __init__(self, paths):
        self.paths = paths
        self.dir = paths.identity_dir
        self.cert = self.dir / "cert.pem"
        self.key = self.dir / "key.pem"
        self.fingerprint = ""
        self.stato = "invalida"
        self.errore = ""
        self.origine = ""     # 'esistente' | 'creata' | 'importata-v3'
        self.carica()

    def carica(self):
        self.dir.mkdir(parents=True, exist_ok=True)
        os.chmod(self.dir, 0o700)
        have = (self.cert.exists(), self.key.exists())
        if all(have):
            try:
                self.fingerprint = _check_pair(self.cert, self.key)
                self.stato, self.errore, self.origine = "ok", "", "esistente"
            except Exception as e:  # noqa: BLE001
                self.stato, self.errore = "invalida", "Identità non valida: %s" % e
            return
        if any(have):
            self.stato = "invalida"
            self.errore = "Identità incompleta: manca certificato o chiave."
            return
        if self._importa_v3():
            return
        self._genera()
        self.origine = "creata"

    def _importa_v3(self):
        old = self.paths.legacy_cache
        try:
            fp = _check_pair(old / "cert.pem", old / "key.pem")
        except Exception:  # noqa: BLE001
            return False
        shutil.copy2(old / "cert.pem", self.cert)
        _write_private(self.key, (old / "key.pem").read_bytes())
        self.fingerprint, self.stato, self.errore, self.origine = fp, "ok", "", "importata-v3"
        return True

    def _genera(self):
        key = rsa.generate_private_key(65537, 2048)
        name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "Pixel Hub")])
        now = datetime.datetime.now(datetime.timezone.utc)
        cert = (x509.CertificateBuilder().subject_name(name).issuer_name(name)
                .public_key(key.public_key()).serial_number(x509.random_serial_number())
                .not_valid_before(now - datetime.timedelta(days=1))
                .not_valid_after(now + datetime.timedelta(days=3650))
                .sign(key, hashes.SHA256()))
        self.cert.write_bytes(cert.public_bytes(serialization.Encoding.PEM))
        _write_private(self.key, key.private_bytes(
            serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()))
        self.fingerprint = fingerprint_der(cert.public_bytes(serialization.Encoding.DER))
        self.stato, self.errore = "ok", ""

    def rigenera(self):
        """Azione esplicita dell'utente: conserva i vecchi file in una cartella di backup."""
        bk = self.dir / ("backup-" + time.strftime("%Y%m%d-%H%M%S"))
        bk.mkdir(mode=0o700)
        for p in (self.cert, self.key):
            if p.exists():
                shutil.move(str(p), str(bk / p.name))
        self._genera()
        self.origine = "creata"
        return self.fingerprint

    def server_context(self):
        c = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        c.minimum_version = ssl.TLSVersion.TLSv1_2
        c.load_cert_chain(str(self.cert), str(self.key))
        return c

    def client_context(self):
        # I certificati LocalSend sono autofirmati: l'identità è l'impronta SHA-256,
        # verificata a parte da client.py contro quella attesa.
        c = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        c.minimum_version = ssl.TLSVersion.TLSv1_2
        c.check_hostname = False
        c.verify_mode = ssl.CERT_NONE
        try:
            c.load_cert_chain(str(self.cert), str(self.key))
        except (OSError, ssl.SSLError):
            pass
        return c
