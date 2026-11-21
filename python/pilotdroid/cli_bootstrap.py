"""pilotdroid-cli — lazy bootstrap for the Rust CLI binary.

The PyPI wheel ships no binary (pure-python whl). On first run this entry
point downloads the release asset for the current platform from
github.com/mofanx/pilotdroid/releases, verifies it against the release's
SHA256SUMS.txt, caches it under ``~/.cache/pilotdroid/<tag>/``, then
``os.execv``'s the real binary with argv passthrough — so
``pilotdroid-cli setup``, ``pilotdroid-cli serve`` etc. work as if the
binary were installed directly.

Refuses to exec on sha256 mismatch or any download/integrity failure.
"""

import hashlib
import importlib.metadata
import os
import platform
import stat
import sys
import urllib.request

REPO = "mofanx/pilotdroid"
TIMEOUT = 60

# (platform.system(), normalized machine) -> release asset basename
_ASSETS = {
    ("Linux", "x86_64"): "pilotdroid-cli-linux-x86_64",
    ("Darwin", "arm64"): "pilotdroid-cli-macos-arm64",
    ("Windows", "amd64"): "pilotdroid-cli-windows-x86_64.exe",
}


def _machine():
    m = platform.machine().lower()
    return {"aarch64": "arm64", "x64": "x86_64", "amd64": "amd64"}.get(m, m)


def _asset_name():
    sys_name, mach = platform.system(), _machine()
    name = _ASSETS.get((sys_name, mach))
    if name is None:
        supported = ", ".join(sorted(a for a in _ASSETS.values()))
        raise RuntimeError(
            f"no pilotdroid-cli release binary for {sys_name}/{mach} — "
            f"supported assets: {supported}. Build from source instead: "
            "github.com/mofanx/pilotdroid (cargo build -p pilotdroid-cli)")
    return name


def _tag():
    try:
        return "v" + importlib.metadata.version("pilotdroid")
    except importlib.metadata.PackageNotFoundError:
        return "latest"  # dev tree without install


def _url(tag, name):
    if tag == "latest":
        return f"https://github.com/{REPO}/releases/latest/download/{name}"
    return f"https://github.com/{REPO}/releases/download/{tag}/{name}"


def _fetch(url):
    with urllib.request.urlopen(url, timeout=TIMEOUT) as r:
        return r.read()


def _expected_sha(sums_text, asset):
    for line in sums_text.splitlines():
        parts = line.split()
        if len(parts) == 2 and parts[1] == asset:
            return parts[0]
    raise RuntimeError(
        f"{asset} not listed in SHA256SUMS.txt — refusing to run an "
        "unverifiable binary")


def _cache_dir(tag):
    return os.path.join(os.path.expanduser("~"), ".cache", "pilotdroid", tag)


def _sums(tag):
    return _fetch(_url(tag, "SHA256SUMS.txt")).decode()


def _write_verified(path, blob, want, asset):
    got = hashlib.sha256(blob).hexdigest()
    if got != want:
        raise RuntimeError(
            f"sha256 mismatch for {asset}: got {got}, want {want} — "
            "refusing to exec an unverified binary")
    tmp = path + ".tmp"
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(tmp, "wb") as f:
        f.write(blob)
    os.chmod(tmp, os.stat(tmp).st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    os.replace(tmp, path)


def _ensure_binary():
    asset, tag = _asset_name(), _tag()
    path = os.path.join(_cache_dir(tag), asset)
    if os.path.exists(path):
        # Cache hit still verifies: a re-uploaded/corrupted release asset
        # must not be exec'd. Offline (SUMS unreachable) → run the cache.
        try:
            want = _expected_sha(_sums(tag), asset)
        except Exception:
            return path
        with open(path, "rb") as f:
            if hashlib.sha256(f.read()).hexdigest() == want:
                return path
        os.remove(path)  # stale/corrupt cache → fall through to re-download
        try:
            return _finish(path, asset, tag, want)
        except Exception as e:
            raise RuntimeError(
                f"cached {asset} failed sha256 re-check and re-download "
                f"failed: {e}")
    try:
        blob = _fetch(_url(tag, asset))
        want = _expected_sha(_sums(tag), asset)
    except Exception as e:
        raise RuntimeError(
            f"failed to download {asset} ({tag}): {e}. Check network/proxy, "
            "or download manually from "
            f"github.com/{REPO}/releases and put it at {path}")
    _write_verified(path, blob, want, asset)
    return path


def _finish(path, asset, tag, want):
    blob = _fetch(_url(tag, asset))
    _write_verified(path, blob, want, asset)
    return path


def main():
    try:
        path = _ensure_binary()
    except RuntimeError as e:
        sys.exit(f"pilotdroid-cli bootstrap: {e}")
    # Windows argv[0] is cosmetic; execv semantics replace this process.
    os.execv(path, [path] + sys.argv[1:])


if __name__ == "__main__":
    main()
