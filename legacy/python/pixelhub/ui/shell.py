"""Shell GTK4/libadwaita: finestra WebKit con l'interfaccia HTML esistente.

Niente server HTTP e niente polling: la pagina chiama il motore tramite un canale di
messaggi nativo di WebKit, e il motore le *spinge* lo stato quando cambia qualcosa.
"""
import json
import os
import threading
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
gi.require_version("WebKit", "6.0")
gi.require_version("JavaScriptCore", "6.0")
from gi.repository import Adw, Gdk, Gio, GLib, Gtk  # noqa: E402
from gi.repository import JavaScriptCore as JSC  # noqa: E402
from gi.repository import WebKit  # noqa: E402

from .. import APP_ID, VERSION, desktop  # noqa: E402
from ..bus import log  # noqa: E402
from ..engine import Engine  # noqa: E402
from ..util import dimensione  # noqa: E402
from .api import ASYNC_METHODS, UiApi  # noqa: E402

UI_DIR = Path(__file__).resolve().parent
STATIC = {"index.html": "text/html; charset=utf-8", "icon.svg": "image/svg+xml"}


class HubApp(Adw.Application):
    def __init__(self, background=False, paths=None):
        super().__init__(application_id=APP_ID,
                         flags=Gio.ApplicationFlags.HANDLES_OPEN)
        self.background = background
        self.paths = paths
        self.engine = None
        self.api = None
        self.window = None
        self.web = None
        self.loaded = False
        self.first = True
        self.tenuta = False
        self.pending_files = []
        self.pool = ThreadPoolExecutor(max_workers=4, thread_name_prefix="ui")
        self._armed = False
        self._stale = False
        self._lock = threading.Lock()

    # ------------------------------------------------------------------ ciclo di vita
    def do_startup(self):
        Adw.Application.do_startup(self)
        self.hold()                     # il motore resta attivo anche a finestra chiusa
        self.engine = Engine(self.paths)
        self.api = UiApi(self.engine, self)
        self.engine.notifier = lambda kind, d: GLib.idle_add(self._notify, kind, d)
        self.engine.bus.subscribe(self._on_event)
        self.engine.avvia()
        self._azioni()
        self._monitor_rete()
        display = Gdk.Display.get_default()
        if display:
            icons = UI_DIR.parents[3] / "data" / "icons"
            if icons.is_dir():
                Gtk.IconTheme.get_for_display(display).add_search_path(str(icons))

    def do_activate(self):
        if self.first and self.background:
            self.first = False
            return
        self.first = False
        self.mostra()

    def do_open(self, files, n_files, hint):
        self.first = False
        paths = [f.get_path() for f in files if f.get_path()]
        log("open", paths)
        self.mostra()
        self._file_alla_pagina(paths)

    def do_shutdown(self):
        if self.engine:
            self.engine.ferma()
        Adw.Application.do_shutdown(self)

    def esci(self):
        GLib.idle_add(self.quit)

    # ------------------------------------------------------------------ azioni dell'app e notifiche
    def _azioni(self):
        def add(name, cb, param=None):
            a = Gio.SimpleAction.new(name, GLib.VariantType.new(param) if param else None)
            a.connect("activate", cb)
            self.add_action(a)
        add("mostra", lambda *_: self.mostra())
        add("esci", lambda *_: self.esci())
        add("accetta", lambda _a, v: self._decidi(v.get_string(), True), "s")
        add("rifiuta", lambda _a, v: self._decidi(v.get_string(), False), "s")
        add("apri-cartella", lambda *_: desktop.apri(self.engine.settings["cartella"]))
        self.set_accels_for_action("app.esci", ["<Primary>q"])

    def _decidi(self, rid, accetta):
        self.pool.submit(self.engine.decidi, rid, None if accetta else [])
        self.ritira_notifica(rid)

    def ritira_notifica(self, rid):
        GLib.idle_add(lambda: (self.withdraw_notification("req-" + rid), False)[1])

    def _notify(self, kind, d):
        if self.window and self.window.is_active() and kind != "fine":
            return False
        n = None
        if kind == "richiesta":
            n = Gio.Notification.new(d["alias"])
            tot = dimensione(d["size"])
            n.set_body("vuole inviarti un messaggio" if d.get("tipo") == "testo" else
                       "vuole inviarti %s (%s)" % ("un file" if d["n"] == 1 else "%d file" % d["n"], tot))
            n.add_button_with_target("Accetta", "app.accetta", GLib.Variant.new_string(d["id"]))
            n.add_button_with_target("Rifiuta", "app.rifiuta", GLib.Variant.new_string(d["id"]))
            n.set_default_action("app.mostra")
            n.set_priority(Gio.NotificationPriority.HIGH)
            self.send_notification("req-" + d["id"], n)
            return False
        if kind == "fine":
            self.withdraw_notification("req-" + d["id"])
            if d["stato"] == "completata":
                n = Gio.Notification.new("Ricevuto da " + d["alias"])
                n.set_body("%d file" % d["ricevuti"] if d["ricevuti"] != 1 else "1 file")
                n.add_button("Apri cartella", "app.apri-cartella")
            elif d["stato"] in ("parziale", "errore", "scaduta"):
                n = Gio.Notification.new("Ricezione da %s non completata" % d["alias"])
                n.set_body(d.get("errore") or "")
                n.add_button("Apri cartella", "app.apri-cartella")
            n and n.set_default_action("app.mostra")
        elif kind == "messaggio":
            n = Gio.Notification.new("Messaggio da " + d["alias"])
            n.set_body(d["testo"][:120])
            n.set_default_action("app.mostra")
        elif kind == "invio" and d["stato"] in ("completato", "errore", "parziale", "rifiutato"):
            n = Gio.Notification.new({"completato": "Invio completato", "parziale": "Invio parziale",
                                      "rifiutato": "Invio rifiutato", "errore": "Invio non riuscito"}[d["stato"]])
            n.set_body("%s: %s" % (d["alias"], d.get("errore") or "ok") if d.get("errore") else d["alias"])
            n.set_default_action("app.mostra")
        if n:
            self.send_notification(None, n)
        return False

    # ------------------------------------------------------------------ rete e sospensione
    def _monitor_rete(self):
        self._net_timer = 0

        def changed(*_):
            if self._net_timer:
                GLib.source_remove(self._net_timer)
            self._net_timer = GLib.timeout_add_seconds(2, self._rete_assestata)
        Gio.NetworkMonitor.get_default().connect("network-changed", changed)
        try:
            bus = Gio.bus_get_sync(Gio.BusType.SYSTEM, None)

            def sleep_sig(_c, _s, _p, _i, _n, params):
                if not params.unpack()[0]:      # False = ripresa dalla sospensione
                    GLib.timeout_add_seconds(3, self._rete_assestata)
            bus.signal_subscribe("org.freedesktop.login1", "org.freedesktop.login1.Manager", "PrepareForSleep",
                                 "/org/freedesktop/login1", None, Gio.DBusSignalFlags.NONE, sleep_sig)
            self._sys_bus = bus
        except GLib.Error as e:
            log("login1", e)

    def _rete_assestata(self):
        self._net_timer = 0
        self.pool.submit(self.engine.rete_cambiata)
        return False

    # ------------------------------------------------------------------ finestra
    def mostra(self):
        if self.window is None:
            self._crea_finestra()
        self.window.present()
        if self._stale:
            self._on_event("stato", {})

    def _crea_finestra(self):
        w = Adw.ApplicationWindow(application=self, title="Pixel Hub")
        w.set_default_size(1120, 780)
        w.set_size_request(400, 560)
        w.set_icon_name(APP_ID)

        ctx = WebKit.WebContext.get_default()
        ctx.register_uri_scheme("hub", self._schema)
        ctx.get_security_manager().register_uri_scheme_as_secure("hub")
        ucm = WebKit.UserContentManager()
        ucm.register_script_message_handler_with_reply("hub", None)
        ucm.connect("script-message-with-reply-received::hub", self._messaggio)
        web = WebKit.WebView(user_content_manager=ucm)
        st = web.get_settings()
        st.set_enable_developer_extras(bool(os.environ.get("HUB_DEBUG")))
        st.set_enable_back_forward_navigation_gestures(False)
        try:
            st.set_hardware_acceleration_policy(WebKit.HardwareAccelerationPolicy.ALWAYS)
        except AttributeError:
            pass
        web.connect("decide-policy", self._politica)
        web.connect("load-changed", self._caricata)
        web.connect("web-process-terminated", lambda *_: web.load_uri("hub://app/index.html"))
        self.web = web

        over = Gtk.Overlay(child=web)
        maniglia = Gtk.WindowHandle(valign=Gtk.Align.START, height_request=44)
        barra = Gtk.Box(margin_start=10, margin_end=10)
        barra.append(Gtk.WindowControls(side=Gtk.PackType.START, valign=Gtk.Align.CENTER))
        barra.append(Gtk.Box(hexpand=True))
        barra.append(Gtk.WindowControls(side=Gtk.PackType.END, valign=Gtk.Align.CENTER))
        maniglia.set_child(barra)
        over.add_overlay(maniglia)
        w.set_content(over)

        # trascinamento nativo: la pagina riceve i percorsi reali, senza copie intermedie
        drop = Gtk.DropTarget.new(Gdk.FileList, Gdk.DragAction.COPY)
        drop.set_propagation_phase(Gtk.PropagationPhase.CAPTURE)
        drop.connect("enter", lambda *_: (self._js("window.hubDragState&&hubDragState(true)"), Gdk.DragAction.COPY)[1])
        drop.connect("leave", lambda *_: self._js("window.hubDragState&&hubDragState(false)"))
        drop.connect("drop", self._rilascio)
        over.add_controller(drop)

        w.connect("close-request", self._chiusura)
        w.connect("notify::visible", lambda *_: self._visibilita())
        w.connect("notify::is-active", lambda *_: self._visibilita())
        self.window = w
        web.load_uri("hub://app/index.html")

    def _visibilita(self):
        vis = bool(self.window and self.window.get_visible())
        self.engine.android.ui_attiva(vis)
        if vis and self._stale:
            self._on_event("stato", {})

    def _chiusura(self, _w):
        if self.engine.settings["background"]:
            self.window.set_visible(False)
            return True
        self.esci()
        return True

    def _politica(self, _v, decisione, tipo):
        if tipo == WebKit.PolicyDecisionType.NAVIGATION_ACTION:
            uri = decisione.get_navigation_action().get_request().get_uri()
            if not uri.startswith("hub://"):
                decisione.ignore()
                if uri.startswith(("http://", "https://")):
                    desktop.apri(uri)
                return True
        return False

    def _caricata(self, _v, evento):
        if evento == WebKit.LoadEvent.FINISHED:
            self.loaded = True
            if self.pending_files:
                self._file_alla_pagina(self.pending_files)
                self.pending_files = []

    # ------------------------------------------------------------------ schema hub://
    def _schema(self, req):
        uri = req.get_uri()
        path = uri[len("hub://app"):].split("?")[0]
        q = dict(p.split("=", 1) for p in (uri.split("?", 1)[1].split("&") if "?" in uri else []) if "=" in p)
        data, mime = None, None
        if path in ("", "/", "/index.html"):
            name = "index.html"
        else:
            name = path.lstrip("/")
        if name in STATIC:
            try:
                data, mime = (UI_DIR / name).read_bytes(), STATIC[name]
            except OSError:
                pass
        elif name == "anteprima":
            from urllib.parse import unquote
            r = self.api.anteprima(unquote(q.get("path", "")))
            if r:
                data, mime = r
        if data is None:
            req.finish_error(GLib.Error.new_literal(Gio.io_error_quark(), "non trovato", Gio.IOErrorEnum.NOT_FOUND))
            return
        req.finish(Gio.MemoryInputStream.new_from_bytes(GLib.Bytes.new(data)), len(data), mime)

    # ------------------------------------------------------------------ canale messaggi pagina → motore
    def _messaggio(self, _mgr, value, reply):
        try:
            msg = json.loads(value.to_json(0))
            metodo, args = msg.get("m"), msg.get("a") or {}
        except (ValueError, TypeError):
            return False
        jsctx = value.get_context()

        def rispondi(obj):
            def go():
                try:
                    reply.return_value(JSC.Value.new_from_json(jsctx, json.dumps(obj, ensure_ascii=False)))
                except Exception as e:  # noqa: BLE001
                    log("reply", repr(e))
                return False
            GLib.idle_add(go)

        if metodo in ASYNC_METHODS:
            getattr(self, "_" + metodo)(args, rispondi)
        else:
            def work():
                rispondi(self.api.chiama(metodo, args))
            self.pool.submit(work)
        return True

    def _scegli_file(self, args, done):
        d = Gtk.FileDialog(title="Scegli i file da inviare")

        def fine(dlg, res):
            try:
                lst = dlg.open_multiple_finish(res)
                done({"ok": True, "percorsi": [lst.get_item(i).get_path() for i in range(lst.get_n_items())]})
            except GLib.Error:
                done({"ok": False, "percorsi": []})
        d.open_multiple(self.window, None, fine)

    def _scegli_cartella(self, args, done):
        k = "cartella_foto" if args.get("quale") == "foto" else "cartella"
        d = Gtk.FileDialog(title="Scegli una cartella")
        try:
            d.set_initial_folder(Gio.File.new_for_path(self.engine.settings[k]))
        except GLib.Error:
            pass

        def fine(dlg, res):
            try:
                f = dlg.select_folder_finish(res)
            except GLib.Error:
                return done({"ok": False})
            path = f.get_path() if f else None
            if path:
                self.pool.submit(self.engine.aggiorna_impostazioni, {k: path})
            done({"ok": bool(path)})
        d.select_folder(self.window, None, fine)

    # ------------------------------------------------------------------ motore → pagina
    def _js(self, script):
        if self.web and self.loaded:
            self.web.evaluate_javascript(script, -1, None, None, None, None, None)

    def _file_alla_pagina(self, paths):
        if self.loaded:
            self._js("window.hubFile&&hubFile(%s)" % json.dumps(paths))
        else:
            self.pending_files += paths

    def _rilascio(self, _t, value, x, y):
        paths = [f.get_path() for f in value.get_files() if f.get_path()]
        self._js("window.hubDragState&&hubDragState(false)")
        if paths:
            self._js("window.hubDrop&&hubDrop(%s,%d,%d)" % (json.dumps(paths), int(x), int(y)))
        return bool(paths)

    def _on_event(self, topic, data):
        """Chiamata da qualunque thread: accorpa gli eventi e spinge al massimo ~10 volte/s."""
        with self._lock:
            if self._armed:
                return
            self._armed = True
        GLib.idle_add(self._arma)

    def _arma(self):
        GLib.timeout_add(100, self._flush)
        return False

    def _flush(self):
        with self._lock:
            self._armed = False
        if not (self.window and self.loaded and self.window.get_visible()):
            self._stale = True      # nessuno guarda: si aggiorna alla prossima apertura
            return False
        self._stale = False
        self.pool.submit(self._costruisci_e_spingi)
        return False

    def _costruisci_e_spingi(self):
        try:
            js = "window.hubStato&&hubStato(%s)" % json.dumps(self.engine.snapshot(), ensure_ascii=False)
        except Exception as e:  # noqa: BLE001
            log("snapshot", repr(e))
            return
        GLib.idle_add(lambda: (self._js(js), False)[1])


def run(files=(), background=False, paths=None):
    app = HubApp(background, paths)
    return app.run(["pixel-hub"] + list(files))
