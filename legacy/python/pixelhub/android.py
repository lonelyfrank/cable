"""Funzioni Android opzionali (scrcpy e importazione da USB/ADB).

Indipendenti da LocalSend: nessuna di queste funzioni influisce su scoperta, identità
o autorizzazioni dei trasferimenti Wi-Fi.
"""
import collections
import re
import shlex
import shutil
import subprocess
import threading
import time
import uuid
from pathlib import Path

from .bus import log
from .net import run
from .util import commit_unique


def ha(cmd):
    return shutil.which(cmd) is not None


class Android:
    def __init__(self, settings, bus):
        self.settings = settings
        self.bus = bus
        self.stato_ = {"stato": "assente", "info": {}, "errore": ""}
        self.serial = None
        self.imp = {"attivo": False, "fatti": 0, "tot": 0, "msg": ""}
        self.proc = None
        self.connesso = False
        self.log = collections.deque(maxlen=120)
        self.dal = 0
        self.attivo_ui = False
        self._stop = threading.Event()
        self._wake = threading.Event()

    # ------------------------------------------------------------------ monitor
    def start(self):
        threading.Thread(target=self._loop, daemon=True).start()

    def stop(self):
        self._stop.set()
        self._wake.set()
        self.ferma_schermo()

    def ui_attiva(self, on):
        """Il monitor ADB gira solo se la finestra è visibile."""
        self.attivo_ui = on
        if on:
            self._wake.set()

    def _loop(self):
        while not self._stop.is_set():
            if self.attivo_ui and not self.imp["attivo"]:
                try:
                    old = self.stato_
                    self.controlla()
                    if old != self.stato_:
                        self.bus.emit("android")
                except Exception as e:  # noqa: BLE001
                    log("android", repr(e))
            self._wake.wait(5)
            self._wake.clear()

    # ------------------------------------------------------------------ ADB
    def controlla(self):
        self.serial = None
        if not ha("adb"):
            self.stato_ = {"stato": "mancante", "info": {}, "errore": ""}
            return
        rc, out = run(["adb", "devices"], 10)
        devs = [l.split() for l in out.splitlines()
                if len(l.split()) >= 2 and not l.startswith(("List ", "*", "adb:"))]
        devs = [d for d in devs if d[1] in ("device", "unauthorized", "offline", "no")]
        if rc or not devs:
            self.stato_ = {"stato": "assente", "info": {}, "errore": out if rc else ""}
            return
        if len(devs) != 1:
            self.stato_ = {"stato": "errore", "info": {}, "errore":
                           "Sono collegati più dispositivi ADB: lascia collegato solo quello che vuoi usare."}
            return
        serial, status = devs[0][:2]
        if status != "device":
            self.stato_ = {"stato": "fiducia" if status == "unauthorized" else "errore", "info": {},
                           "errore": "Autorizza Debug USB sul telefono." if status == "unauthorized"
                           else "ADB: " + status + ". Ricollega il cavo e controlla i permessi USB."}
            return
        self.serial = serial

        def sh(*a):
            c, v = run(["adb", "-s", serial, "shell", *a], 10)
            return v if c == 0 else ""
        model = sh("getprop", "ro.product.model")
        info = {"DeviceName": model or "Android", "ProductType": model,
                "ProductVersion": sh("getprop", "ro.build.version.release")}
        battery = dict(re.findall(r"^\s*([\w ]+):\s*(.+)$", sh("dumpsys", "battery"), re.M))
        info["BatteryCurrentCapacity"] = battery.get("level", "")
        info["BatteryIsCharging"] = str(battery.get("status") in ("2", "5")).lower()
        lines = sh("df", "-k", "/sdcard").splitlines()
        if len(lines) > 1:
            f = lines[-1].split()
            try:
                info["TotalDiskCapacity"] = str(int(f[1]) * 1024)
                info["AmountDataAvailable"] = str(int(f[3]) * 1024)
            except (ValueError, IndexError):
                pass
        self.stato_ = {"stato": "collegato", "info": info, "errore": ""}

    def riprova(self):
        self.controlla()
        self.bus.emit("android")
        return self.stato_["stato"] == "collegato"

    # ------------------------------------------------------------------ importazione DCIM
    def importa(self):
        if self.imp["attivo"] or self.stato_["stato"] != "collegato" or not self.serial:
            return False
        self.imp = {"attivo": True, "fatti": 0, "tot": 0, "msg": "Leggo DCIM sul telefono…"}
        threading.Thread(target=self._importa, args=(self.serial,), daemon=True).start()
        return True

    def _importa(self, serial):
        dest = Path(self.settings["cartella_foto"]).resolve()
        msg = ""
        try:
            rc, listing = run(["adb", "-s", serial, "shell", "find /sdcard/DCIM -type f -print0"], 60)
            if rc:
                raise OSError(listing)
            paths = [s for s in listing.split("\0") if s.startswith("/sdcard/DCIM/")]
            self.imp.update(tot=len(paths), msg="Controllo e copio foto e video…")
            copied = skipped = 0
            for remote in paths:
                q = (dest / Path(remote).relative_to("/sdcard/DCIM")).resolve()
                if not q.is_relative_to(dest):
                    raise OSError("Percorso DCIM non valido")
                rc, size = run(["adb", "-s", serial, "shell", "stat -c %s " + shlex.quote(remote)], 15)
                if rc or not size.isdigit():
                    raise OSError("Impossibile leggere " + remote)
                size = int(size)
                if q.exists() and q.stat().st_size == size:
                    skipped += 1
                else:
                    q.parent.mkdir(parents=True, exist_ok=True)
                    tmp = q.with_name("." + q.name + "." + uuid.uuid4().hex[:8] + ".part")
                    try:
                        rc, out = run(["adb", "-s", serial, "pull", remote, str(tmp)], 3600)
                        if rc or not tmp.exists() or tmp.stat().st_size != size:
                            raise OSError(out or "Copia incompleta")
                        commit_unique(tmp, q)
                    finally:
                        tmp.unlink(missing_ok=True)
                    copied += 1
                self.imp["fatti"] += 1
                self.bus.emit("android")
            msg = ("Importati %d; già presenti %d." % (copied, skipped)) if paths else \
                "DCIM vuota: si importano solo i file locali, non quelli presenti solo su Google Foto."
        except Exception as e:  # noqa: BLE001
            msg = "Importazione interrotta: " + str(e)[:300]
        finally:
            self.imp.update(attivo=False, msg=msg)
            self.bus.emit("android")

    def apri_file_manager(self):
        try:
            subprocess.Popen(["gio", "open", "computer:///"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            return True
        except OSError:
            return False

    # ------------------------------------------------------------------ scrcpy
    def avvia_schermo(self):
        if self.proc and self.proc.poll() is None:
            return True
        if not ha("scrcpy"):
            return False
        self.controlla()
        if self.stato_["stato"] != "collegato":
            self.log.append(self.stato_.get("errore") or "Collega un telefono e autorizza il debug USB.")
            self.bus.emit("android")
            return False
        argv = ["scrcpy", "--serial", self.serial, "--max-size=1920", "--max-fps=60"]
        if self.settings["schermo_intero"]:
            argv.append("--fullscreen")
        try:
            self.proc = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                         text=True, bufsize=1, errors="replace")
        except OSError as e:
            self.log.append(str(e))
            return False
        self.connesso = False
        self.dal = time.time()
        self.log.append("$ " + " ".join(argv))
        threading.Thread(target=self._leggi, args=(self.proc,), daemon=True).start()
        self.bus.emit("android")
        return True

    def _leggi(self, p):
        for riga in p.stdout:
            riga = riga.rstrip()
            self.log.append(riga)
            low = riga.lower()
            if any(k in low for k in ("texture:", "renderer:")):
                self.connesso = True
            elif any(k in low for k in ("connection closed", "teardown", "client disconnected")):
                self.connesso = False
            self.bus.emit("android")
        if p is self.proc:
            self.connesso = False
            self.log.append("— ricevitore fermato —")
            self.bus.emit("android")

    def ferma_schermo(self):
        p = self.proc
        if p and p.poll() is None:
            p.terminate()
            try:
                p.wait(3)
            except subprocess.TimeoutExpired:
                p.kill()
        self.connesso = False
        self.bus.emit("android")

    # ------------------------------------------------------------------ stato
    def snapshot(self):
        attivo = bool(self.proc and self.proc.poll() is None)
        return {"adb": dict(self.stato_, importa=dict(self.imp), destinazione=self.settings["cartella_foto"]),
                "schermo": {"installato": ha("scrcpy"), "attivo": attivo, "connesso": attivo and self.connesso,
                            "dal": self.dal if attivo else 0, "log": list(self.log)[-40:]}}
