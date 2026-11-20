"""pilotdroid — uiautomator2-compatible Python client for the pilotdroid agent.

    import pilotdroid
    d = pilotdroid.connect(port=17932, serial="<adb serial>")
    d(text="OK").click()
"""

from .u2shim import connect, Device  # noqa: F401
