#!/usr/bin/env python3
"""live-view: agent frame stream → MJPEG in the browser.

Serves http://127.0.0.1:HTTP/ — open it and watch the device screen live
(the ch=1 media pipe at ~5fps@0.5scale, same frames `stream_read` emits).

    python3 spike/e2e/live-view.py [serial] [control_port] [http_port]

Prereqs: agent running (pilotdroid-cli agent start), control forward
`adb forward tcp:PORT localabstract:pilotdroid_agent`. Ctrl-C stops the
stream and removes the media forward it created.
"""
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

sys.path.insert(0, __file__.rsplit("/", 2)[0] + "/../agent/scripts")
import agent_client as ac  # noqa: E402

INDEX = (b"<html><body style='margin:0;background:#111'>"
         b"<img src='/mjpeg' style='width:100%;height:auto'>"
         b"</body></html>")
BOUNDARY = b"--pdframe"


class Latest:
    def __init__(self):
        self.jpg = None
        self.seq = -1
        self.cond = threading.Condition()

    def put(self, jpg, seq):
        with self.cond:
            self.jpg, self.seq = jpg, seq
            self.cond.notify_all()

    def wait_next(self, last_seq, timeout=15):
        with self.cond:
            self.cond.wait_for(lambda: self.seq != last_seq, timeout)
            return self.jpg, self.seq


def producer(serial, cport, mport, latest, stop):
    a = ac.Agent(cport).connect()
    a.stream_start(fps=5, scale=0.5, quality=70)
    adb = ["adb"] + (["-s", serial] if serial else [])
    subprocess.run(adb + ["forward", f"tcp:{mport}",
                          "localabstract:pilotdroid_agent_media"],
                   capture_output=True)
    ms = ac.media_connect(mport)
    try:
        while not stop.is_set():
            fr = ac.read_frame(ms)
            if fr is None:
                break
            _ch, rtype, _flags, seq, p = fr
            if rtype == 0x0001:  # FRAME
                latest.put(p["data"], seq)
    finally:
        ms.close()
        try:
            a.stream_stop()
        except Exception:
            pass
        a.close()
        subprocess.run(adb + ["forward", "--remove", f"tcp:{mport}"],
                       capture_output=True)


class Handler(BaseHTTPRequestHandler):
    latest = None

    def do_GET(self):
        if self.path in ("/", "/index.html"):
            body = INDEX
            self.send_response(200)
            self.send_header("Content-Type", "text/html")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        if self.path == "/mjpeg":
            self.send_response(200)
            self.send_header("Content-Type",
                             "multipart/x-mixed-replace; boundary=pdframe")
            self.end_headers()
            seq = -1
            try:
                while True:
                    jpg, seq = self.latest.wait_next(seq)
                    if jpg is None:
                        continue
                    self.wfile.write(BOUNDARY + b"\r\nContent-Type: "
                                     b"image/jpeg\r\n\r\n" + jpg + b"\r\n")
                    self.wfile.flush()
            except (BrokenPipeError, ConnectionResetError):
                pass
            return
        self.send_error(404)

    def log_message(self, *_):
        pass


def main():
    serial = sys.argv[1] if len(sys.argv) > 1 else None
    cport = int(sys.argv[2]) if len(sys.argv) > 2 else 17932
    hport = int(sys.argv[3]) if len(sys.argv) > 3 else 8080
    mport = cport + 1
    latest = Latest()
    Handler.latest = latest
    stop = threading.Event()
    th = threading.Thread(target=producer,
                          args=(serial, cport, mport, latest, stop),
                          daemon=True)
    th.start()
    print(f"live-view: http://127.0.0.1:{hport}/ "
          f"(control tcp:{cport}, media tcp:{mport}) — Ctrl-C to stop")
    try:
        ThreadingHTTPServer(("127.0.0.1", hport), Handler).serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        stop.set()
        th.join(timeout=3)


if __name__ == "__main__":
    main()
