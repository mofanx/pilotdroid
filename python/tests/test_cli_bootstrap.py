"""cli_bootstrap unit tests — download/verify/cache/exec paths, all network
and exec fully mocked. Run: python -m unittest discover -s python/tests -q
"""

import hashlib
import os
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from pilotdroid import cli_bootstrap as cb  # noqa: E402

FAKE_BIN = b"\x7fELF fake-pilotdroid-cli-binary"
FAKE_SHA = hashlib.sha256(FAKE_BIN).hexdigest()
ASSET = "pilotdroid-cli-linux-x86_64"


def _sums(entry=FAKE_SHA, name=ASSET):
    return f"{entry}  {name}\n"


class _LinuxPlatform(unittest.TestCase):
    def setUp(self):
        self._sys = mock.patch.object(cb.platform, "system", return_value="Linux")
        self._mach = mock.patch.object(cb.platform, "machine", return_value="x86_64")
        self._tag = mock.patch.object(cb, "_tag", return_value="v0.6.0")
        self.tmp = tempfile.TemporaryDirectory()
        self._home = mock.patch.dict(os.environ, {"HOME": self.tmp.name})
        for p in (self._sys, self._mach, self._tag, self._home):
            p.start()
            self.addCleanup(p.stop)
        self.addCleanup(self.tmp.cleanup)
        # expanduser caches nothing on linux; HOME patch suffices.

    def test_download_verify_cache_exec(self):
        calls = []

        def fetch(url):
            calls.append(url)
            if url.endswith("SHA256SUMS.txt"):
                return _sums().encode()
            return FAKE_BIN

        with mock.patch.object(cb, "_fetch", side_effect=fetch), \
                mock.patch.object(cb.os, "execv") as ex:
            with mock.patch.object(sys, "argv", ["pilotdroid-cli", "setup", "-s", "X"]):
                cb.main()
        # downloaded asset + sums (2 fetches)
        self.assertEqual(len(calls), 2)
        self.assertIn("v0.6.0", calls[0])
        # exec'd the cached binary with argv passthrough
        path, argv = ex.call_args[0]
        self.assertTrue(path.endswith(ASSET))
        self.assertEqual(argv, [path, "setup", "-s", "X"])
        self.assertTrue(os.access(path, os.X_OK))
        # second run: cache hit → fetches ONLY SHA256SUMS (re-verify),
        # no binary re-download
        calls.clear()
        with mock.patch.object(cb, "_fetch", side_effect=fetch), \
                mock.patch.object(cb.os, "execv") as ex2:
            cb.main()
        self.assertEqual(calls, [u for u in calls if u.endswith("SHA256SUMS.txt")])
        self.assertEqual(len(calls), 1)
        self.assertEqual(ex2.call_args[0][0], path)

    def test_corrupt_cache_redownloads(self):
        # seed a corrupted cache file, then run — must re-download binary
        cache_dir = os.path.join(self.tmp.name, ".cache", "pilotdroid", "v0.6.0")
        os.makedirs(cache_dir)
        bad = os.path.join(cache_dir, ASSET)
        with open(bad, "wb") as f:
            f.write(b"tampered")

        calls = []

        def fetch(url):
            calls.append(url)
            if url.endswith("SHA256SUMS.txt"):
                return _sums().encode()
            return FAKE_BIN

        with mock.patch.object(cb, "_fetch", side_effect=fetch), \
                mock.patch.object(cb.os, "execv") as ex:
            cb.main()
        # SUMS (cache check) + binary (re-download) fetched
        self.assertEqual(len(calls), 2)
        path, argv = ex.call_args[0]
        with open(path, "rb") as f:
            self.assertEqual(hashlib.sha256(f.read()).hexdigest(), FAKE_SHA)

    def test_offline_cache_hit_still_execs(self):
        # valid cached binary + network down → exec anyway (offline)
        cache_dir = os.path.join(self.tmp.name, ".cache", "pilotdroid", "v0.6.0")
        os.makedirs(cache_dir)
        good = os.path.join(cache_dir, ASSET)
        with open(good, "wb") as f:
            f.write(FAKE_BIN)

        with mock.patch.object(cb, "_fetch", side_effect=OSError("down")), \
                mock.patch.object(cb.os, "execv") as ex:
            cb.main()
        self.assertEqual(ex.call_args[0][0], good)

    def test_sha_mismatch_refuses_exec(self):
        def fetch(url):
            if url.endswith("SHA256SUMS.txt"):
                return _sums(entry="0" * 64).encode()
            return FAKE_BIN

        with mock.patch.object(cb, "_fetch", side_effect=fetch), \
                mock.patch.object(cb.os, "execv") as ex, \
                self.assertRaises(SystemExit) as cm:
            cb.main()
        ex.assert_not_called()
        self.assertIn("sha256 mismatch", str(cm.exception))
        # nothing cached
        self.assertFalse(os.path.exists(
            os.path.join(self.tmp.name, ".cache", "pilotdroid")))

    def test_asset_missing_from_sums(self):
        def fetch(url):
            if url.endswith("SHA256SUMS.txt"):
                return _sums(name="pilotdroid-cli-other").encode()
            return FAKE_BIN

        with mock.patch.object(cb, "_fetch", side_effect=fetch), \
                self.assertRaises(SystemExit) as cm:
            cb.main()
        self.assertIn("not listed in SHA256SUMS", str(cm.exception))

    def test_network_failure_actionable(self):
        with mock.patch.object(cb, "_fetch",
                               side_effect=OSError("no route")), \
                self.assertRaises(SystemExit) as cm:
            cb.main()
        self.assertIn("failed to download", str(cm.exception))
        self.assertIn("releases", str(cm.exception))

    def test_unsupported_platform_lists_assets(self):
        with mock.patch.object(cb.platform, "system", return_value="FreeBSD"), \
                self.assertRaises(SystemExit) as cm:
            cb.main()
        self.assertIn("no pilotdroid-cli release binary", str(cm.exception))
        self.assertIn("cargo build -p pilotdroid-cli", str(cm.exception))


if __name__ == "__main__":
    unittest.main()
