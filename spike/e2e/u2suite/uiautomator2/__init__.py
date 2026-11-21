# M18 seam: verbatim openatx/uiautomator2 suite against u2shim.
# The ONLY harness adaptation left: connect() pins the adb serial —
# the dev box has two devices attached and the upstream suite was
# written for a single emulator. All u2-API gaps from M17 (settings,
# exceptions submodule, implicit waits, ...) are now implemented in
# u2shim itself — no shim patching happens here.
import os, sys
sys.path.insert(0, os.path.join(os.path.dirname(__file__),
                                '..', '..', '..', '..', 'compat'))
import u2shim as _u2shim
sys.modules['uiautomator2'] = _u2shim

_orig_connect = _u2shim.connect
def _connect_pinned(port=None, serial=None):
    return _orig_connect(port=port,
                         serial=serial
                         or os.environ.get('PILOTDROID_SERIAL',
                                           'ba0d072d'))
_u2shim.connect = _connect_pinned

from u2shim import *  # noqa
