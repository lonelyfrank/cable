"""Ponte fra la pagina web dell'interfaccia e il motore (indipendente da GTK, quindi testabile)."""
import mimetypes
import re
from pathlib import Path

from .. import desktop
from ..util import tipo_file

IP_RE = re.compile(r"^\d{1,3}(\.\d{1,3}){3}$")
# chiamate che richiedono una finestra di dialogo: le gestisce la shell GTK
ASYNC_METHODS = ("scegli_file", "scegli_cartella")


class UiApi:
    def __init__(self, engine, shell=None):
        self.e = engine
        self.shell = shell

    # ------------------------------------------------------------ sicurezza dei percorsi
    def consentito(self, p):
        """Solo cartella di ricezione, foto importate: la pagina non può aprire altro."""
        try:
            p = Path(p).resolve()
        except (OSError, RuntimeError):
            return None
        for k in ("cartella", "cartella_foto"):
            try:
                if p.is_relative_to(Path(self.e.settings[k]).resolve()):
                    return p
            except (OSError, RuntimeError):
                pass
        return None

    def anteprima(self, path):
        """(bytes, mime) per le miniature delle foto ricevute, oppure None."""
        p = self.consentito(path)
        if not p or not p.is_file() or tipo_file(p.name) != "foto" or p.suffix.lower() in (".heic", ".heif") \
                or p.stat().st_size > 40_000_000:
            return None
        return p.read_bytes(), mimetypes.guess_type(p.name)[0] or "image/jpeg"

    # ------------------------------------------------------------ dispatcher
    def chiama(self, metodo, a):
        fn = getattr(self, "m_" + str(metodo), None)
        if fn is None or not isinstance(a, dict):
            return {"ok": False, "errore": "Chiamata sconosciuta"}
        try:
            r = fn(a)
        except Exception as e:  # noqa: BLE001
            from ..bus import log
            log("api", metodo, repr(e))
            return {"ok": False, "errore": "Operazione non riuscita"}
        return r if isinstance(r, dict) else {"ok": bool(r)}

    # ------------------------------------------------------------ stato
    def m_stato(self, a):
        return self.e.snapshot()

    def m_diagnostica(self, a):
        return {"ok": True, "testo": self.e.diagnostica()}

    # ------------------------------------------------------------ dispositivi e invio
    def m_cerca(self, a):
        ip = a.get("ip")
        if ip and not (isinstance(ip, str) and IP_RE.match(ip)):
            return {"ok": False, "errore": "Indirizzo IP non valido"}
        return {"ok": True, "trovati": self.e.cerca(ip or None)}

    def m_invia(self, a):
        paths = [str(p) for p in a.get("percorsi") or []]
        jid, err = self.e.invia(str(a.get("fp", "")), paths, a.get("testo"))
        return {"ok": jid is not None, "id": jid, "errore": err}

    def m_annulla_invio(self, a):
        return self.e.sender.annulla(str(a.get("id", "")))

    def m_pin_invio(self, a):
        return self.e.sender.fornisci_pin(str(a.get("id", "")), a.get("pin"))

    def m_preferito(self, a):
        return self.e.preferito(str(a.get("fp", "")), bool(a.get("on")))

    def m_dimentica(self, a):
        return self.e.dimentica(str(a.get("fp", "")))

    # ------------------------------------------------------------ ricezione
    def m_decidi(self, a):
        ids = a.get("ids")
        if ids is not None and not isinstance(ids, list):
            return False
        ok = self.e.decidi(str(a.get("id", "")), None if ids is None else [str(i) for i in ids])
        if self.shell:
            self.shell.ritira_notifica(str(a.get("id", "")))
        return ok

    def m_annulla_ricezione(self, a):
        return self.e.receiver.annulla(str(a.get("id", "")))

    def m_msg_rimuovi(self, a):
        self.e.receiver.rimuovi_messaggio(str(a.get("id", "")))
        return True

    # ------------------------------------------------------------ impostazioni
    def m_conf(self, a):
        return {"ok": True, "aggiornati": self.e.aggiorna_impostazioni(a)}

    def m_pin(self, a):
        pin = a.get("pin")
        ok = self.e.imposta_pin(pin if pin else None)
        return {"ok": ok, "errore": "" if ok else "Il PIN deve avere da 4 a 32 caratteri"}

    def m_ripristina(self, a):
        return {"ok": True, "backup": self.e.ripristina_impostazioni()}

    def m_rigenera_identita(self, a):
        return {"ok": self.e.rigenera_identita()}

    def m_riprova_server(self, a):
        return self.e.riprova_server()

    # ------------------------------------------------------------ cartelle e file
    def m_apri(self, a):
        p = self.consentito(a.get("path", ""))
        return bool(p and p.exists() and desktop.apri(p))

    def m_mostra(self, a):
        p = self.consentito(a.get("path", ""))
        return bool(p and p.exists() and desktop.mostra(p))

    def m_apri_cartella(self, a):
        k = "cartella_foto" if a.get("quale") == "foto" else "cartella"
        p = Path(self.e.settings[k])
        p.mkdir(parents=True, exist_ok=True)
        return desktop.apri(p)

    # ------------------------------------------------------------ Android (opzionale)
    def m_schermo(self, a):
        if a.get("azione") == "avvia":
            return self.e.android.avvia_schermo()
        self.e.android.ferma_schermo()
        return True

    def m_android_importa(self, a):
        return self.e.android.importa()

    def m_android_apri(self, a):
        return self.e.android.apri_file_manager()

    def m_android_riprova(self, a):
        return self.e.android.riprova()

    # ------------------------------------------------------------ applicazione
    def m_esci(self, a):
        if self.shell:
            self.shell.esci()
        return True

    def m_js_error(self, a):
        from ..bus import log
        log("JS", a.get("m"), a.get("l"))
        return True

    def m_attiva(self, a):
        self.e.android.ui_attiva(bool(a.get("on")))
        return True
