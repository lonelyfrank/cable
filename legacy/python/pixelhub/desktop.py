"""Integrazione con il desktop: cartelle, file manager e avvio automatico."""
import os
import shlex
import subprocess
import sys
from pathlib import Path

from . import APP_ID


def _spawn(argv):
    try:
        subprocess.Popen(argv, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        return True
    except OSError:
        return False


def apri(path):
    return _spawn(["gio", "open", str(path)])


def mostra(path):
    """Evidenzia il file nel file manager; ripiega sull'apertura della cartella."""
    p = Path(path)
    try:
        r = subprocess.run(
            ["gdbus", "call", "--session", "--dest", "org.freedesktop.FileManager1",
             "--object-path", "/org/freedesktop/FileManager1", "--method",
             "org.freedesktop.FileManager1.ShowItems", "['%s']" % p.as_uri(), ""],
            capture_output=True, timeout=5)
        if r.returncode == 0:
            return True
    except (OSError, subprocess.TimeoutExpired):
        pass
    return apri(p.parent)


def comando_avvio():
    """Comando per avviare l'app: quello installato, altrimenti il modulo dal sorgente."""
    from shutil import which
    exe = which("pixel-hub")
    if exe:
        return [exe]
    return [sys.executable, "-m", "pixelhub"]


def autostart_file(paths):
    return paths.home / ".config" / "autostart" / (APP_ID + ".desktop")


def imposta_avvio(paths, attivo):
    f = autostart_file(paths)
    try:
        if attivo:
            f.parent.mkdir(parents=True, exist_ok=True)
            exec_line = " ".join(shlex.quote(a) for a in comando_avvio() + ["--background"])
            f.write_text("[Desktop Entry]\nType=Application\nName=Pixel Hub\nExec=%s\nIcon=%s\n"
                         "X-GNOME-Autostart-enabled=true\nNoDisplay=true\n" % (exec_line, APP_ID))
        elif f.exists():
            f.unlink()
        return True
    except OSError:
        return False


def processi_vecchia_versione():
    """PID di istanze della versione 3.x (script pixel_hub.py), per evitare avvii simultanei."""
    found = []
    me = os.getpid()
    for d in Path("/proc").iterdir():
        if not d.name.isdigit() or int(d.name) == me:
            continue
        try:
            argv = (d / "cmdline").read_bytes().split(b"\0")
        except OSError:
            continue
        if any(a.endswith(b"/pixel-hub/pixel_hub.py") or a == b"pixel_hub.py" for a in argv[:4]):
            found.append(int(d.name))
    return found
