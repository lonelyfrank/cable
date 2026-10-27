import os
import unittest

from pixelhub.protocol import Limits, ProtocolError, is_text_message, parse_peer_info, parse_prepare_upload
from pixelhub.util import PathError, Progress, commit_unique, prepare_target, safe_parts, truncate_name
from tests.helpers import TmpCase


class SafeNames(unittest.TestCase):
    def test_traversal_and_separators(self):
        self.assertEqual(safe_parts("../../etc/passwd"), ["etc", "passwd"])
        self.assertEqual(safe_parts("a\\..\\b/./c"), ["a", "b", "c"])
        self.assertEqual(safe_parts("/abs/path.txt"), ["abs", "path.txt"])

    def test_empty_and_control(self):
        self.assertEqual(safe_parts(""), ["file"])
        self.assertEqual(safe_parts("..") , ["file"])
        self.assertEqual(safe_parts("a\x00b\n.txt"), ["ab.txt"])

    def test_unicode_and_length(self):
        self.assertEqual(safe_parts("città/è già.txt"), ["città", "è già.txt"])
        long = "é" * 300 + ".jpg"
        out = truncate_name(long)
        self.assertLessEqual(len(out.encode()), 200)
        self.assertTrue(out.endswith(".jpg"))


class Targets(TmpCase):
    def test_symlink_component_rejected(self):
        root = self.tmp / "root"
        root.mkdir()
        outside = self.tmp / "outside"
        outside.mkdir()
        (root / "link").symlink_to(outside)
        with self.assertRaises(PathError):
            prepare_target(root, ["link", "x.txt"])
        self.assertEqual(list(outside.iterdir()), [])

    def test_symlink_destination_rejected(self):
        root = self.tmp / "root"
        root.mkdir()
        (root / "f.txt").symlink_to(self.tmp / "victim")
        with self.assertRaises(PathError):
            prepare_target(root, ["f.txt"])

    def test_creates_subdirs(self):
        root = self.tmp / "root"
        root.mkdir()
        p = prepare_target(root, ["a", "b", "c.txt"])
        self.assertEqual(p, root.resolve() / "a" / "b" / "c.txt")
        self.assertTrue((root / "a" / "b").is_dir())

    def test_commit_never_overwrites(self):
        d = self.tmp
        (d / "f.txt").write_text("vecchio")
        for i in range(3):
            t = d / f"t{i}"
            t.write_text(f"nuovo{i}")
            commit_unique(t, d / "f.txt")
        self.assertEqual((d / "f.txt").read_text(), "vecchio")
        self.assertEqual((d / "f (1).txt").read_text(), "nuovo0")
        self.assertEqual((d / "f (2).txt").read_text(), "nuovo1")
        self.assertEqual((d / "f (3).txt").read_text(), "nuovo2")
        self.assertFalse((d / "t0").exists())


def body(**files):
    return {"info": {"alias": "x", "fingerprint": "A" * 64}, "files": files}


def meta(**kw):
    d = {"id": "i", "fileName": "a.txt", "size": 3, "fileType": "text/plain"}
    d.update(kw)
    return d


class Validation(unittest.TestCase):
    def bad(self, b, lim=Limits()):
        with self.assertRaises(ProtocolError) as c:
            parse_prepare_upload(b, lim)
        self.assertEqual(c.exception.status, 400)

    def test_ok(self):
        info, files = parse_prepare_upload(body(i=meta(sha256="a" * 64)))
        self.assertEqual(files["i"].size, 3)
        self.assertEqual(files["i"].sha256, "a" * 64)

    def test_rejects(self):
        self.bad(None)
        self.bad({"info": {}, "files": {}})
        self.bad({"files": {"i": meta()}})
        self.bad(body(i=meta(size=-1)))
        self.bad(body(i=meta(size="3")))
        self.bad(body(i=meta(size=True)))
        self.bad(body(i=meta(size=2 ** 60)))
        self.bad(body(i=meta(fileName="")))
        self.bad(body(i=meta(fileName="x" * 2000)))
        self.bad(body(i=meta(sha256="zz")))
        self.bad(body(i=meta(preview=5)))
        self.bad(body(i=meta(preview="x" * 70000)))
        self.bad(body(i="nope"))

    def test_limits(self):
        lim = Limits(max_files=2, max_total_size=10)
        self.bad(body(a=meta(id="a"), b=meta(id="b"), c=meta(id="c")), lim)
        self.bad(body(a=meta(id="a", size=6), b=meta(id="b", size=6)), lim)

    def test_text_message_detection(self):
        _, f = parse_prepare_upload(body(i=meta(preview="ciao")))
        self.assertTrue(is_text_message(f))
        _, f = parse_prepare_upload(body(i=meta(), j=meta(id="j", preview="x")))
        self.assertFalse(is_text_message(f))

    def test_peer_info(self):
        ok = {"alias": "Pixel\x00", "fingerprint": "a" * 64, "port": 53317, "protocol": "https"}
        self.assertEqual(parse_peer_info(ok)["fp"], "A" * 64)
        self.assertIsNone(parse_peer_info(dict(ok, protocol="http")))
        self.assertIsNone(parse_peer_info(dict(ok, fingerprint="short")))
        self.assertIsNone(parse_peer_info(dict(ok, port=0)))
        self.assertIsNone(parse_peer_info(dict(ok, port=True)))
        self.assertIsNone(parse_peer_info("x"))


class ProgressTest(unittest.TestCase):
    def test_speed_needs_window(self):
        p = Progress()
        p.add(100)
        self.assertIsNone(p.speed())


if __name__ == "__main__":
    unittest.main()
