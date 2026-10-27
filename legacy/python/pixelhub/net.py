"""Informazioni sulla rete locale (letture rapide, senza dipendenze extra)."""
import re
import subprocess


def run(argv, timeout=5):
    try:
        p = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
        return p.returncode, (p.stdout + p.stderr).strip()
    except (OSError, subprocess.TimeoutExpired) as e:
        return 1, str(e)


def local_ips():
    rc, out = run(["ip", "-4", "-o", "addr", "show", "scope", "global"])
    ips = []
    for line in out.splitlines() if rc == 0 else []:
        p = line.split()
        if len(p) > 3:
            ips.append((p[1], p[3].split("/")[0]))
    return ips


def gateway():
    rc, out = run(["ip", "-4", "route", "show", "default"])
    m = re.search(r"via (\d+\.\d+\.\d+\.\d+)", out) if rc == 0 else None
    return m.group(1) if m else None


def network_name():
    rc, out = run(["nmcli", "-t", "-f", "NAME,TYPE", "connection", "show", "--active"])
    for line in out.splitlines() if rc == 0 else []:
        n, _, t = line.rpartition(":")
        if "wireless" in t or "ethernet" in t:
            return n.replace("\\:", ":")
    return ""
