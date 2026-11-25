package pilotdroid;

import android.app.UiAutomation;
import android.graphics.Bitmap;
import android.graphics.Rect;
import android.net.LocalServerSocket;
import android.net.LocalSocket;
import android.os.Build;
import android.os.Bundle;
import android.os.Looper;
import android.os.SystemClock;
import android.view.InputDevice;
import android.view.InputEvent;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.accessibility.AccessibilityNodeInfo;

import java.io.ByteArrayOutputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.lang.reflect.Constructor;
import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;

/**
 * pilotdroid device agent — M2: single app_process, BATCH/wait_until/QUERY/
 * TEXT/SHOT-scale/NOACK per docs/02-protocol.md §4 (finalized contract).
 *
 * <p>Serve: localabstract:pilotdroid_agent, PD01 18-byte frames + msgpack.
 *
 * <p>Launch:
 * <pre>
 *   CLASSPATH=/data/local/tmp/pilotdroid/agent.jar setsid env \
 *     app_process /data/local/tmp/pilotdroid pilotdroid.Agent
 * </pre>
 */
public final class Agent {
    private static final String SOCKET_NAME = "pilotdroid_agent";
    private static final String MEDIA_SOCKET = "pilotdroid_agent_media";
    private static final String AGENT_VER = "0.7.3";
    private static final int PROTO_VER = 0;

    // capability bitmap — normative spec is pilotdroid-proto `caps` (bit-for-bit)
    private static final int CAP_INJECT = 1, CAP_DUMP = 2, CAP_SHOT = 4,
            CAP_QUERY = 8, CAP_BATCH = 16, CAP_STREAM = 32, CAP_TEXT = 64,
            CAP_MPTR = 128, CAP_WATCH = 256,
            CAP_WATCH_ASYNC = 512, CAP_APP_INFO = 1024;

    // opcodes
    private static final int T_HELLO = 0x0001, T_HELLO_ACK = 0x0002,
            T_PING = 0x0003, T_PONG = 0x0004, T_INJECT = 0x0010,
            T_INJECT_RESULT = 0x0011, T_DUMP = 0x0020, T_QUERY = 0x0021,
            T_SHOT = 0x0030, T_BATCH = 0x0040, T_BATCH_RESULT = 0x0041,
            T_STREAM = 0x0050, T_FRAME = 0x0001, T_WATCH_HIT = 0x0002,
            T_TEXT = 0x0060, T_WATCH = 0x0070, T_WATCH_ARM = 0x0071,
            T_WATCH_DISARM = 0x0072, T_APP_INFO = 0x0080, T_STOP = 0x00F0;
    private static final int CH_MEDIA = 1;
    private static final int F_RESP = 0x0001, F_MORE = 0x0002,
            F_ERR = 0x0004, F_NOACK = 0x0008;

    private Agent() {}

    public static void main(String[] args) throws Exception {
        try {
            Looper.prepareMainLooper();
        } catch (Throwable ignore) {
        }
        UiAutomation uia = connectUiAutomation();
        Dumper dumper = new Dumper(uia);
        Injector injector = new Injector(uia);
        // toast detection (u2 parity): TYPE_NOTIFICATION_STATE_CHANGED
        // events — toast posts carry no ParcelableData while real
        // notifications do; push {type:"toast"} on the media channel.
        try {
            uia.setOnAccessibilityEventListener(ev -> {
                if (ev == null || ev.getEventType() !=
                        android.view.accessibility.AccessibilityEvent
                                .TYPE_NOTIFICATION_STATE_CHANGED) return;
                Map<String, Object> p = new LinkedHashMap<>();
                p.put("type", "toast");
                CharSequence pkg = ev.getPackageName();
                p.put("pkg", pkg == null ? "" : pkg.toString());
                p.put("has_parcel", ev.getParcelableData() != null);
                List<String> tx = new ArrayList<>();
                for (CharSequence t : ev.getText())
                    if (t != null) tx.add(t.toString());
                p.put("text", tx);
                Streamer.uiEvent(p);
            });
        } catch (Throwable t) {
            System.out.println("a11y event listener unavailable: " + t);
        }

        LocalServerSocket srv = new LocalServerSocket(SOCKET_NAME);
        // media channel (§4.8): second localabstract, isolated from control
        try {
            final LocalServerSocket msrv = new LocalServerSocket(MEDIA_SOCKET);
            new Thread(() -> {
                for (;;) {
                    LocalSocket s;
                    try {
                        s = msrv.accept();
                    } catch (Throwable t) {
                        return;        // server socket dead — agent exiting
                    }
                    try {
                        Streamer.attach(s);
                    } catch (Throwable t) {
                        // A dead/stale backlog conn must never kill the
                        // accept thread — every later conn would sit in
                        // the backlog un-served (t4 ghost mechanism).
                        System.out.println("media attach dropped: " + t);
                    }
                }
            }, "pd-media-accept").start();
        } catch (Throwable t) {
            System.out.println("media socket unavailable: " + t);
        }
        System.out.println("pilotdroid-agent " + AGENT_VER + " listening localabstract:"
                + SOCKET_NAME);
        // takeScreenshot only works on the thread that owns the UiAutomation
        // connection (main). Sessions + accept run on workers; main loops to
        // marshal capture calls (shotSync).
        mainHandler = new android.os.Handler(Looper.getMainLooper());
        new Thread(() -> {
            for (;;) {
                try {
                    final LocalSocket c = srv.accept();
                    // single-client semantics: a new control conn replaces the
                    // previous one (closes its socket -> its session read fails
                    // -> worker exits). Cross-process op interleave is thus
                    // impossible; last connector wins.
                    synchronized (Agent.class) {
                        if (curCtrl != null) {
                            // close() alone does NOT interrupt another
                            // thread's blocked read on the same fd — the
                            // syscall pins the file description and the old
                            // session keeps working. shutdown() delivers FIN
                            // -> peer EOF -> session read fails -> exits.
                            lastReplaced = curCtrl;
                            try { curCtrl.shutdownInput(); } catch (Throwable ignore) {}
                            try { curCtrl.shutdownOutput(); } catch (Throwable ignore) {}
                            try { curCtrl.close(); } catch (Throwable ignore) {}
                        }
                        curCtrl = c;
                    }
                    new Thread(() -> {
                        try {
                            new Session(c, uia, injector, dumper).run();
                        } catch (Throwable t) {
                            synchronized (Agent.class) {
                                if (c == lastReplaced) {
                                    lastReplaced = null;
                                    System.out.println("session ended: replaced");
                                } else {
                                    System.out.println("session ended: " + t);
                                }
                            }
                        } finally {
                            synchronized (Agent.class) {
                                if (curCtrl == c) curCtrl = null;
                            }
                            try { c.close(); } catch (Throwable ignore) {}
                        }
                    }, "pd-session").start();
                } catch (Throwable t) {
                    return;
                }
            }
        }, "pd-accept").start();
        Looper.loop();
    }

    private static android.os.Handler mainHandler;
    /** The single live control conn; replaced on new accept (see accept loop). */
    private static LocalSocket curCtrl;
    /** Conn most recently killed by replacement — lets the session's end
     *  log distinguish "replaced" from a natural EOF/error. */
    private static volatile LocalSocket lastReplaced;

    /** takeScreenshot bound to the connection thread (main); marshal when
     *  called from a worker (session op thread or stream producer). */
    private static Bitmap shotSync(final UiAutomation uia) {
        if (Looper.myLooper() == Looper.getMainLooper())
            synchronized (uia) { return uia.takeScreenshot(); }
        final Bitmap[] out = new Bitmap[1];
        final java.util.concurrent.CountDownLatch done =
                new java.util.concurrent.CountDownLatch(1);
        mainHandler.post(() -> {
            try { synchronized (uia) { out[0] = uia.takeScreenshot(); } }
            catch (Throwable t) { System.out.println("cap on main threw: " + t); }
            done.countDown();
        });
        boolean ok;
        try { ok = done.await(3, java.util.concurrent.TimeUnit.SECONDS); }
        catch (InterruptedException e) { return null; }
        if (!ok) System.out.println("cap: main marshal timeout");
        return out[0];
    }

    // ---------- infra -------------------------------------------------------

    private static UiAutomation connectUiAutomation() throws Exception {
        return connectUiAutomation(Looper.getMainLooper());
    }

    private static UiAutomation connectUiAutomation(Looper l) throws Exception {
        Class<?> connCls = Class.forName("android.app.UiAutomationConnection");
        Object conn = connCls.getDeclaredConstructor().newInstance();
        Constructor<UiAutomation> ctor = UiAutomation.class.getDeclaredConstructor(
                Looper.class, Class.forName("android.app.IUiAutomationConnection"));
        ctor.setAccessible(true);
        UiAutomation u = ctor.newInstance(l, conn);
        Method connect = UiAutomation.class.getDeclaredMethod("connect", int.class);
        connect.setAccessible(true);
        connect.invoke(u, 1); // FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES
        return u;
    }

    /** Op-level failure carrying a contract code (BATCH result.code / ERROR.code). */
    private static final class OpError extends Exception {
        final String code;
        OpError(String code, String message) { super(message); this.code = code; }
    }

    /** One client connection: frame in -> dispatch -> frame out. */
    private static final class Session {
        private final DataInputStream in;
        private final DataOutputStream out;
        private final UiAutomation uia;
        private final Injector injector;
        private final Dumper dumper;
        private volatile boolean stop;
        /** Cap on armed watchers per session — each polls the full tree
         *  under the global a11y lock; unbounded arming starves session
         *  queries (§4.9). Exceeding this is an ERR_LIMIT ack error. */
        private static final int MAX_WATCHERS = 8;
        /** Armed async watchers owned by this session — killed on session
         *  end (§4.8-style session binding). */
        private final List<Watcher> watchers = new ArrayList<>();

        Session(LocalSocket s, UiAutomation uia, Injector inj, Dumper dmp)
                throws Exception {
            this.in = new DataInputStream(s.getInputStream());
            this.out = new DataOutputStream(s.getOutputStream());
            this.uia = uia;
            this.injector = inj;
            this.dumper = dmp;
        }

        void run() throws Exception {
            try {
                byte[] magic = new byte[4];
                while (!stop) {
                in.readFully(magic);
                if (magic[0] != 'P' || magic[1] != 'D' || magic[2] != '0' || magic[3] != '1') {
                    return; // not our protocol — drop
                }
                in.readUnsignedByte();          // ver
                int ch = in.readUnsignedByte();
                int type = in.readUnsignedShort();
                int flags = in.readUnsignedShort();
                long reqId = in.readInt() & 0xffffffffL;
                int plen = in.readInt();
                if (plen < 0 || plen > 64 * 1024 * 1024) return;
                if (ch != 0) return;            // control-only socket (§4.6)
                byte[] payload = new byte[plen];
                in.readFully(payload);
                Object req = Msgpack.read(payload);
                respond(type, flags, reqId, req);
                }
            } finally {
                // §4.8 session binding: control disconnect = implicit stop
                Streamer.ctrlClosed(this);
                for (Watcher w : watchers) w.disarm();
                watchers.clear();
            }
        }

        private void respond(int reqType, int flags, long reqId, Object req)
                throws Exception {
            boolean noack = (flags & F_NOACK) != 0;
            // §4.5: NOACK is only legal on fire-and-forget ops; on
            // response-ops it is a protocol error (error must not be swallowed).
            if (noack && reqType != T_INJECT && reqType != T_TEXT) {
                write(reqType, reqId, F_RESP | F_ERR,
                        errPayload("PROTO", "NOACK illegal on response opcode"));
                return;
            }
            int rtype;
            Object payload;
            boolean err = false;
            try {
                switch (reqType) {
                    case T_HELLO:   rtype = T_HELLO_ACK;      payload = hello();          break;
                    case T_PING:    rtype = T_PONG;
                        payload = map("ts", SystemClock.uptimeMillis());                  break;
                    case T_INJECT:  rtype = T_INJECT_RESULT;  payload = inject(req);      break;
                    case T_DUMP:    rtype = T_DUMP;           payload = dumper.xml();     break;
                    case T_QUERY:   rtype = T_QUERY;          payload = query(req);       break;
                    case T_SHOT:    rtype = T_SHOT;           payload = shot(req);        break;
                    case T_TEXT:    rtype = T_TEXT;           payload = text(req);        break;
                    case T_BATCH:   rtype = T_BATCH_RESULT;   payload = batch(req);       break;
                    case T_STREAM:  rtype = T_STREAM;         payload = streamCtrl(req);  break;
                    case T_WATCH:   rtype = T_WATCH;          payload = watch(req);       break;
                    case T_WATCH_ARM:   rtype = T_WATCH_ARM;   payload = watchArm(req);   break;
                    case T_WATCH_DISARM:rtype = T_WATCH_DISARM;payload = watchDisarm(req);break;
                    case T_APP_INFO:    rtype = T_APP_INFO;    payload = appInfo();       break;
                    case T_STOP:    rtype = T_STOP;
                        payload = map("bye", true); stop = true;                          break;
                    default:
                        throw new OpError("UNSUPPORTED",
                                "unsupported opcode 0x" + Integer.toHexString(reqType));
                }
            } catch (OpError t) {
                rtype = reqType; err = true;
                payload = errPayload(t.code, String.valueOf(t.getMessage()));
            } catch (Throwable t) {
                rtype = reqType; err = true;
                payload = errPayload("INTERNAL", String.valueOf(t));
            }
            if (noack) {
                if (stop) System.exit(0);
                return; // fire-and-forget honored
            }
            write(rtype, reqId, F_RESP | (err ? F_ERR : 0), payload);
            if (stop) System.exit(0);
        }

        private void write(int rtype, long reqId, int flags, Object payload)
                throws Exception {
            out.write("PD01".getBytes("US-ASCII"));
            out.writeByte(PROTO_VER);
            out.writeByte(0); // control channel
            out.writeShort(rtype);
            out.writeShort(flags);
            out.writeInt((int) reqId);
            byte[] body = Msgpack.write(payload);
            out.writeInt(body.length);
            out.write(body);
            out.flush();
        }

        private Map<String, Object> hello() {
            Map<String, Object> m = new LinkedHashMap<>();
            Rect b = dumper.displayBounds();
            m.put("agent", "pilotdroid");
            m.put("ver", AGENT_VER);
            m.put("proto", PROTO_VER);
            m.put("sdk", Build.VERSION.SDK_INT);
            m.put("abi", Build.SUPPORTED_ABIS.length > 0 ? Build.SUPPORTED_ABIS[0] : "?");
            List<Object> disp = new ArrayList<>();
            disp.add(b.width()); disp.add(b.height());
            m.put("display", disp);
            m.put("caps", CAP_INJECT | CAP_DUMP | CAP_SHOT | CAP_QUERY | CAP_BATCH
                    | CAP_STREAM | CAP_TEXT | CAP_MPTR | CAP_WATCH
                    | CAP_WATCH_ASYNC | CAP_APP_INFO);
            m.put("inject", injector.backend()); // "im" | "uia" | "im+uia-fallback"
            return m;
        }

        private Map<String, Object> inject(Object req) throws Exception {
            Map<?, ?> r = req instanceof Map ? (Map<?, ?>) req : new LinkedHashMap<>();
            boolean waitFinish = bool(r.get("wait_finish"), false);
            // §4.7 backend force: "im"|"uia"; forced failure is ERR_BACKEND,
            // never a silent downgrade to the other channel.
            String force = str(r.get("backend"), null);
            if (force != null) {
                if (!"im".equals(force) && !"uia".equals(force))
                    throw new OpError("ERR_BACKEND", "unknown backend " + force);
                if (!injector.has(force))
                    throw new OpError("ERR_BACKEND", "backend " + force + " unavailable");
            }
            long t0 = System.nanoTime();
            int events;
            boolean ok;
            Object kind = r.get("kind");
            if (kind != null) {
                // wire contract: {kind:"Tap"|"Swipe"|"Key",
                //                 points:[{action,x,y,t_ms}]|keycode, wait_finish}
                if ("Key".equals(String.valueOf(kind))) {
                    events = injector.key((int) num(r.get("keycode"), 0),
                            (int) num(r.get("meta"), 0), waitFinish, force);
                    ok = events == 2;
                } else {
                    List<?> pts = r.get("points") instanceof List
                            ? (List<?>) r.get("points") : new ArrayList<>();
                    events = injector.points(pts, waitFinish, force);
                    ok = events == pts.size() && events > 0;
                }
            } else {
                // legacy flat schema used by agent_client.py / verify-agent.sh
                String op = str(r.get("op"), "tap");
                if ("swipe".equals(op)) {
                    events = injector.swipe(
                            (float) num(r.get("x1"), 0), (float) num(r.get("y1"), 0),
                            (float) num(r.get("x2"), 0), (float) num(r.get("y2"), 0),
                            (int) num(r.get("duration_ms"), 300),
                            (int) num(r.get("steps"), 10), waitFinish, force);
                    ok = events > 0;
                } else {
                    events = injector.tap((float) num(r.get("x"), 0),
                            (float) num(r.get("y"), 0), waitFinish, force);
                    ok = events == 2;
                }
            }
            if (force != null && !ok)
                throw new OpError("ERR_BACKEND",
                        "forced backend " + force + " failed: " + events + " events");
            long cost = System.nanoTime() - t0;
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("ok", ok);
            m.put("events", events);
            m.put("ms", cost / 1e6);   // wire contract field
            m.put("cost_ns", cost);    // observability extra (serde ignores)
            m.put("backend", force != null ? force : injector.backend());
            return m;
        }

        private Map<String, Object> query(Object req) {
            Map<?, ?> r = req instanceof Map ? (Map<?, ?>) req : new LinkedHashMap<>();
            Selector sel = new Selector(r.get("selector"));
            AccessibilityNodeInfo hit = dumper.find(sel);
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("found", hit != null);
            if (hit != null) {
                m.put("node", Dumper.nodeInfo(hit, attrs(r.get("attrs"))));
                hit.recycle();
            }
            return m;
        }

        private static Set<String> attrs(Object o) {
            if (!(o instanceof List)) return null;
            Set<String> s = new LinkedHashSet<>();
            for (Object a : (List<?>) o) s.add(String.valueOf(a));
            return s;
        }

        private Map<String, Object> text(Object req) throws Exception {
            Map<?, ?> r = req instanceof Map ? (Map<?, ?>) req : new LinkedHashMap<>();
            String s = str(r.get("text"), "");
            boolean clear = bool(r.get("clear"), false);
            long t0 = System.nanoTime();
            AccessibilityNodeInfo edit = dumper.focusedEditable();
            if (edit == null) throw new OpError("NO_FOCUS", "no focused editable node");
            boolean ok;
            try {
                if (clear) setText(edit, "");
                ok = setText(edit, s);
            } finally {
                edit.recycle();
            }
            if (!ok) throw new OpError("ERR_SET_TEXT", "ACTION_SET_TEXT returned false");
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("ok", true);
            m.put("chars", s.length());
            m.put("cost_ns", System.nanoTime() - t0);
            return m;
        }

        private static boolean setText(AccessibilityNodeInfo n, String s) {
            Bundle b = new Bundle();
            b.putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, s);
            return n.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, b);
        }

        private Map<String, Object> shot(Object req) {
            Map<?, ?> r = req instanceof Map ? (Map<?, ?>) req : new LinkedHashMap<>();
            int quality = (int) num(r.get("quality"), 80);
            float scale = (float) num(r.get("scale"), 1.0);
            if (scale <= 0f || scale > 1f) scale = 1f;
            long s0 = System.nanoTime();
            Bitmap bmp = shotSync(uia);
            if (bmp == null) throw new IllegalStateException("takeScreenshot null");
            Bitmap outBmp = bmp;
            if (scale < 1f) {
                outBmp = Bitmap.createScaledBitmap(bmp,
                        Math.max(1, Math.round(bmp.getWidth() * scale)),
                        Math.max(1, Math.round(bmp.getHeight() * scale)), true);
            }
            long s1 = System.nanoTime();
            ByteArrayOutputStream bos = new ByteArrayOutputStream(1 << 16);
            outBmp.compress(Bitmap.CompressFormat.JPEG, quality, bos);
            long s2 = System.nanoTime();
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("format", "jpeg");
            m.put("take_ns", s1 - s0);     // observability: capture+scale
            m.put("enc_ns", s2 - s1);      // observability: jpeg encode
            m.put("w", outBmp.getWidth());
            m.put("h", outBmp.getHeight());
            m.put("scale", scale);
            m.put("data", bos.toByteArray());
            outBmp.recycle();
            if (outBmp != bmp) bmp.recycle();
            return m;
        }

        private Map<String, Object> streamCtrl(Object req) throws Exception {
            Map<?, ?> r = req instanceof Map ? (Map<?, ?>) req : new LinkedHashMap<>();
            String op = str(r.get("op"), "start");
            if ("stop".equals(op)) return Streamer.stop(this);
            String fmt = str(r.get("format"), "jpeg");
            if (!"jpeg".equals(fmt))
                throw new OpError("ERR_STREAM_FMT", "format " + fmt + " not in v0 (jpeg only)");
            return Streamer.start(this, uia,
                    (int) num(r.get("fps"), 5),
                    (float) num(r.get("scale"), 0.5),
                    (int) num(r.get("quality"), 70));
        }

        // ---------- BATCH (docs/02 §4) -------------------------------------

        private Map<String, Object> batch(Object req) {
            Map<?, ?> r = req instanceof Map ? (Map<?, ?>) req : new LinkedHashMap<>();
            List<?> ops = r.get("ops") instanceof List ? (List<?>) r.get("ops")
                    : new ArrayList<>();
            boolean stopOnErr = !"continue".equals(str(r.get("fail_policy"), "stop"));
            long deadlineNs = r.get("deadline_ms") instanceof Number
                    ? ((Number) r.get("deadline_ms")).longValue() * 1_000_000L : -1;
            long t0 = System.nanoTime();
            List<Object> results = new ArrayList<>(ops.size());
            boolean aborted = false;
            for (int i = 0; i < ops.size(); i++) {
                Map<String, Object> res = new LinkedHashMap<>();
                res.put("index", i);
                long ot = System.nanoTime();
                if (aborted) {
                    res.put("code", "ABORTED");
                    res.put("message", "skipped: earlier op failed");
                    res.put("cost_ns", 0L);
                    results.add(res);
                    continue;
                }
                if (deadlineNs > 0 && System.nanoTime() - t0 > deadlineNs) {
                    res.put("code", "DEADLINE");
                    res.put("message", "batch deadline exceeded");
                    res.put("cost_ns", 0L);
                    results.add(res);
                    aborted = true;
                    continue;
                }
                String name = "";
                Object spec = null;
                if (ops.get(i) instanceof Map) {
                    Map<?, ?> op = (Map<?, ?>) ops.get(i);
                    for (Map.Entry<?, ?> e : op.entrySet()) {
                        name = String.valueOf(e.getKey());
                        spec = e.getValue();
                        break; // single-key map
                    }
                }
                try {
                    Map<String, Object> data = execOp(name, spec, r);
                    res.put("code", "OK");
                    res.put("message", "");
                    res.put("cost_ns", System.nanoTime() - ot);
                    if (data != null) res.put("data", data);
                } catch (OpError e) {
                    res.put("code", e.code);
                    res.put("message", String.valueOf(e.getMessage()));
                    res.put("cost_ns", System.nanoTime() - ot);
                    aborted = stopOnErr;
                } catch (Throwable e) {
                    res.put("code", "ERR_INTERNAL");
                    res.put("message", String.valueOf(e));
                    res.put("cost_ns", System.nanoTime() - ot);
                    aborted = stopOnErr;
                }
                results.add(res);
            }
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("results", results);
            m.put("total_ns", System.nanoTime() - t0);
            return m;
        }

        /** Execute one batch member by its single key name. */
        private Map<String, Object> execOp(String name, Object spec, Map<?, ?> ctx)
                throws Exception {
            switch (name) {
                case "inject":     return inject(spec);
                case "query":      return query(spec);
                case "dump":       return dumper.xml();
                case "screenshot": return shot(spec);
                case "text":       return text(spec);
                case "wait_until": return waitUntil(spec, ctx);
                default:
                    throw new OpError("UNSUPPORTED", "batch op " + name);
            }
        }

        /** wait_until: poll the selector every poll_ms until timeout. */
        private Map<String, Object> waitUntil(Object spec, Map<?, ?> batchCtx)
                throws Exception {
            Map<?, ?> r = spec instanceof Map ? (Map<?, ?>) spec : new LinkedHashMap<>();
            Selector sel = new Selector(r.get("selector"));
            long timeoutMs = (long) num(r.get("timeout_ms"), 5000);
            long pollMs = (long) num(r.get("poll_ms"),
                    num(batchCtx.get("poll_ms"), 20));
            long t0 = SystemClock.uptimeMillis();
            Map<String, Object> m = new LinkedHashMap<>();
            for (;;) {
                AccessibilityNodeInfo hit = dumper.find(sel);
                if (hit != null) {
                    m.put("found", true);
                    m.put("node", Dumper.nodeInfo(hit));
                    hit.recycle();
                    return m;
                }
                if (SystemClock.uptimeMillis() - t0 >= timeoutMs) {
                    throw new OpError("TIMEOUT",
                            "selector " + sel + " not matched within " + timeoutMs + "ms");
                }
                Thread.sleep(Math.max(1, pollMs));
            }
        }

        /** WATCH op (§4.9): poll a selector inside the agent; when it
         *  appears, optionally perform an action — the whole loop stays in
         *  the device, so detection latency is one tree-walk, not a host
         *  round-trip per poll. {selector, action:{type:"click"|"none"},
         *  timeout_ms, poll_ms} -> {matched, waited_ms, action_ok?, node?} */
        private Map<String, Object> watch(Object req) throws Exception {
            Map<?, ?> r = req instanceof Map ? (Map<?, ?>) req : new LinkedHashMap<>();
            Selector sel = new Selector(r.get("selector"));
            long timeoutMs = (long) num(r.get("timeout_ms"), 10000);
            long pollMs = Math.max(20, (long) num(r.get("poll_ms"), 150));
            Map<?, ?> act = r.get("action") instanceof Map
                    ? (Map<?, ?>) r.get("action") : new LinkedHashMap<>();
            String type = str(act.get("type"), "click");
            if (!"click".equals(type) && !"none".equals(type))
                throw new OpError("ERR_ARGS", "watch action.type must be click|none");
            long t0 = SystemClock.uptimeMillis();
            Map<String, Object> m = new LinkedHashMap<>();
            for (;;) {
                AccessibilityNodeInfo hit = dumper.find(sel);
                if (hit != null) {
                    m.put("matched", true);
                    m.put("waited_ms", SystemClock.uptimeMillis() - t0);
                    if ("click".equals(type)) {
                        Rect b = new Rect();
                        hit.getBoundsInScreen(b);
                        int ev = injector.tap(b.centerX(), b.centerY(),
                                true, null);
                        m.put("action_ok", ev == 2);
                    }
                    m.put("node", Dumper.nodeInfo(hit));
                    hit.recycle();
                    return m;
                }
                if (SystemClock.uptimeMillis() - t0 >= timeoutMs) {
                    m.put("matched", false);
                    m.put("waited_ms", SystemClock.uptimeMillis() - t0);
                    return m;
                }
                Thread.sleep(pollMs);
            }
        }

        /** WATCH_ARM — arm an async watcher: a device-side thread polls the
         *  selector; on hit it runs the action and pushes a WATCH_HIT frame
         *  on the media socket (ch=1, §4.8 ui_event). Returns immediately —
         *  the control conn stays free for parallel ops (u2 watcher parity).
         *  Fire-once semantics; killed on session end. */
        private Map<String, Object> watchArm(Object req) throws Exception {
            Map<?, ?> r = req instanceof Map ? (Map<?, ?>) req : new LinkedHashMap<>();
            Selector sel = new Selector(r.get("selector"));
            // poll floor 50ms: each poll is a full-tree DFS under the
            // global a11y lock — tighter polling starves session queries.
            long pollMs = Math.max(50, (long) num(r.get("poll_ms"), 100));
            long timeoutMs = (long) num(r.get("timeout_ms"), 0); // 0 = no timeout
            Map<?, ?> act = r.get("action") instanceof Map
                    ? (Map<?, ?>) r.get("action") : new LinkedHashMap<>();
            String type = str(act.get("type"), "click");
            if (!"click".equals(type) && !"none".equals(type))
                throw new OpError("ERR_ARGS", "watch action.type must be click|none");
            synchronized (watchers) {
                if (watchers.size() >= MAX_WATCHERS)
                    throw new OpError("ERR_LIMIT",
                            "too many armed watchers (max " + MAX_WATCHERS + ")");
            }
            Watcher w = new Watcher(Watcher.NEXT_ID.getAndIncrement(),
                    this, sel, type, pollMs, timeoutMs);
            synchronized (watchers) { watchers.add(w); }
            w.start();
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("armed", true);
            m.put("watch_id", w.id);
            return m;
        }

        private Map<String, Object> watchDisarm(Object req) {
            Map<?, ?> r = req instanceof Map ? (Map<?, ?>) req : new LinkedHashMap<>();
            int id = (int) num(r.get("watch_id"), -1);
            Watcher hit = null;
            synchronized (watchers) {
                for (Watcher w : watchers) if (w.id == id) { hit = w; break; }
                if (hit != null) watchers.remove(hit);
            }
            if (hit != null) hit.disarm();
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("disarmed", hit != null);
            return m;
        }

        void dropWatcher(Watcher w) {
            synchronized (watchers) { watchers.remove(w); }
        }

        /** APP_INFO — foreground-app lookup entirely inside UiAutomation:
         *  the focused window's root package. No `dumpsys` spawn — ~ms vs
         *  the ~500ms host-adb path the shim used to take. `activity` is
         *  honestly null: accessibility windows don't expose activity
         *  names (u2 gets it via dumpsys — we decline the spawn cost). */
        private Map<String, Object> appInfo() {
            String pkg = null;
            synchronized (uia) { try {
                for (android.view.accessibility.AccessibilityWindowInfo w
                        : uia.getWindows()) {
                    if (!w.isFocused()) continue;
                    AccessibilityNodeInfo root = w.getRoot();
                    if (root != null) {
                        CharSequence p = root.getPackageName();
                        if (p != null) pkg = p.toString();
                        root.recycle();
                    }
                    if (pkg != null) break;
                }
            } catch (Throwable ignore) {}
            if (pkg == null) {
                AccessibilityNodeInfo r = null;
                try {
                    r = uia.getRootInActiveWindow();
                    if (r != null && r.getPackageName() != null)
                        pkg = r.getPackageName().toString();
                } catch (Throwable ignore) {
                } finally {
                    if (r != null) r.recycle();
                }
            } }
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("pkg", pkg);
            m.put("activity", null);
            Rect b = dumper.displayBounds();
            List<Object> disp = new ArrayList<>();
            disp.add(b.width()); disp.add(b.height());
            m.put("display", disp);
            m.put("sdk", Build.VERSION.SDK_INT);
            return m;
        }
    }

    /** Async watcher (§4.9, CAP_WATCH_ASYNC): device-side poll thread —
     *  detection latency = one tree-walk; on hit it acts then pushes a
     *  WATCH_HIT ui_event on the media socket. Fire-once. */
    private static final class Watcher implements Runnable {
        static final java.util.concurrent.atomic.AtomicInteger NEXT_ID =
                new java.util.concurrent.atomic.AtomicInteger(1);
        final int id;
        private final Session owner;
        private final Selector sel;
        private final String action;
        private final long pollMs, timeoutMs;
        private volatile boolean live = true;
        private Thread th;

        Watcher(int id, Session owner, Selector sel, String action,
                long pollMs, long timeoutMs) {
            this.id = id; this.owner = owner; this.sel = sel;
            this.action = action; this.pollMs = pollMs;
            this.timeoutMs = timeoutMs;
        }

        void start() {
            th = new Thread(this, "pd-watch-" + id);
            th.start();
        }

        synchronized void disarm() {
            live = false;
            if (th != null) th.interrupt();
        }

        public void run() {
            long t0 = SystemClock.uptimeMillis();
            while (live) {
                try {
                    AccessibilityNodeInfo hit = owner.dumper.find(sel);
                    if (hit != null) {
                        Map<String, Object> ev = new LinkedHashMap<>();
                        ev.put("watch_id", id);
                        ev.put("matched", true);
                        ev.put("waited_ms", SystemClock.uptimeMillis() - t0);
                        if ("click".equals(action)) {
                            Rect b = new Rect();
                            hit.getBoundsInScreen(b);
                            // waitFinish=false — a sync inject holds
                            // uia.mLock for the whole gesture-wait (~3s
                            // when the target window is transitioning),
                            // serializing every a11y read incl. session
                            // QUERYs. Async means async: dispatch and go.
                            int n = owner.injector.tap(b.centerX(),
                                    b.centerY(), false, null);
                            ev.put("action_ok", n == 2);
                        }
                        ev.put("node", Dumper.nodeInfo(hit));
                        hit.recycle();
                        Streamer.uiEvent(ev);
                        break;
                    }
                    if (timeoutMs > 0
                            && SystemClock.uptimeMillis() - t0 >= timeoutMs) {
                        Map<String, Object> ev = new LinkedHashMap<>();
                        ev.put("watch_id", id);
                        ev.put("matched", false);
                        ev.put("waited_ms", SystemClock.uptimeMillis() - t0);
                        Streamer.uiEvent(ev);
                        break;
                    }
                    Thread.sleep(pollMs);
                } catch (InterruptedException e) {
                    break;
                } catch (Throwable t) {
                    break; // a dead uia/socket must not leak the thread
                }
            }
            owner.dropWatcher(this);
        }
    }

    // ---------- selector (docs/02 §4.3) --------------------------------------

    /** Selector = {text?, id?, desc?, class?, scrollable?, pkg?, index?} — AND
     *  semantics; index is the nth-match ordinal (default 0). */
    private static final class Selector {
        private final String text, id, desc, cls, pkg;
        private final int index;
        private final Boolean scrollable;

        Selector(Object spec) {
            Map<?, ?> m = spec instanceof Map ? (Map<?, ?>) spec : new LinkedHashMap<>();
            this.text = str0(m.get("text"));
            this.id = str0(m.get("id"));
            this.desc = str0(m.get("desc"));
            this.cls = str0(m.get("class"));
            this.pkg = str0(m.get("pkg"));
            this.index = m.get("index") instanceof Number
                    ? ((Number) m.get("index")).intValue() : 0;
            this.scrollable = m.get("scrollable") instanceof Boolean
                    ? (Boolean) m.get("scrollable") : null;
        }

        private static String str0(Object o) { return o == null ? null : String.valueOf(o); }

        boolean match(AccessibilityNodeInfo n) {
            if (text != null && !text.equals(cs(n.getText()))) return false;
            if (desc != null && !desc.equals(cs(n.getContentDescription()))) return false;
            if (id != null) {
                String rid = cs(n.getViewIdResourceName());
                // full "pkg:id/name" equality, or bare "name" -> :id/ suffix
                if (!id.equals(rid) && !rid.endsWith(":id/" + id)) return false;
            }
            if (cls != null) {
                String cn = cs(n.getClassName());
                // full qualified equality, or bare simple name -> ".name" suffix
                if (!cls.equals(cn) && !cn.endsWith("." + cls)) return false;
            }
            if (scrollable != null && scrollable != n.isScrollable()) return false;
            if (pkg != null && !pkg.equals(cs(n.getPackageName()))) return false;
            return true;
        }

        private static String cs(CharSequence s) { return s == null ? "" : s.toString(); }

        public String toString() {
            return "{text=" + text + ",id=" + id + ",desc=" + desc + ",class=" + cls
                    + ",scrollable=" + scrollable + ",pkg=" + pkg + ",index=" + index + "}";
        }
    }

    // ---------- media channel frame stream (§4.8) ----------------------------

    /** JPEG frame pusher on the second localabstract socket. Producer
     *  captures+encodes into a capacity-1 queue; a writer thread drains it
     *  to the media conn — a slow reader can never stall capture or the
     *  control channel (backpressure = drop). */
    private static final class Streamer {
        private static volatile Session owner;
        private static volatile boolean running;
        private static Thread producer, writer;
        private static java.util.concurrent.ArrayBlockingQueue<byte[]> queue;
        private static int frames, dropped, seq;
        private static LocalSocket curSock;
        private static DataOutputStream curOut;
        /** Queued reader while a stream already owns curSock — newest
         *  wins. A stale backlog conn (t4: killed host leaves conns
         *  queued on the media socket; FIFO accept can promote a ghost
         *  ahead of the real reader) no longer gets the real reader
         *  busy-rejected: it waits here and is promoted by mediaDead
         *  the moment the ghost's writes fail. */
        private static LocalSocket resvSock;
        private static DataOutputStream resvOut;
        /** Stream generation: a write failure reported by a previous
         *  generation's writer must never halt the new stream — the
         *  dirty-kill race (mediaDead arriving after the next start()
         *  killed the fresh stream, t4 residual). volatile: writeLoop
         *  reads it in its exit condition outside the monitor. */
        private static volatile int gen;
        /** True once the current curSock has accepted at least one
         *  successful write — a conn that dies without ever writing is
         *  a backlog ghost (t4): drop it and keep waiting for the real
         *  reader instead of halting the stream. A conn that wrote ok
         *  and then dies is a genuine reader loss -> halt (§4.8). */
        private static boolean curWrote;
        /** Watch-mode (non-streaming) held conns — the ui_event push
         *  sinks while no stream runs (§4.9 watchers hold the pending
         *  media socket). A capped set, not a single pend conn: a one
         *  shot push write into a half-dead adb-forward pipe can return
         *  success (adbd absorbs it before reaping), so promote+retry
         *  on a single reserve can still eat the hit. Writing to EVERY
         *  held conn makes the hit reach any live reader; a conn whose
         *  write fails is the corpse judge (t4 curWrote equivalent —
         *  it distinguishes ghost-vs-reader for the log only) and is
         *  dropped. Oldest evicted at cap — backlog ghosts are always
         *  the oldest conns. */
        private static final class WatchConn {
            LocalSocket s; DataOutputStream o; boolean wrote;
            WatchConn(LocalSocket s, DataOutputStream o) {
                this.s = s; this.o = o;
            }
        }
        private static final java.util.ArrayDeque<WatchConn> watchConns =
                new java.util.ArrayDeque<>();
        private static final int WATCH_CONN_CAP = 4;
        /** Hits that found NO live watch conn — stashed and flushed to
         *  the next attaching watch conn (t4): covers the arm-attach
         *  race (armed watcher fires before the agent-side accept
         *  registers the media conn) and the all-corpse window after a
         *  dirty kill. Bounded + expires; a hit written to at least one
         *  conn is considered delivered (a silently-absorbed write
         *  into a half-dead forward can't be detected host-blind). */
        private static final java.util.ArrayDeque<Map<String, Object>>
                pendingHits = new java.util.ArrayDeque<>();
        private static final int PENDING_HIT_CAP = 8;
        private static final long PENDING_HIT_TTL_MS = 15_000;
        private static long lastHitDropTs;
        private static int sFps, sQuality;
        private static float sScale;
        private static boolean nullWarned;
        private static int capNulls;
        private static long capNsSum, encNsSum;

        /** New media connection: pending (pre-start), promoted stream
         *  reader, or reserve behind the current reader — at most one
         *  conn serving, one waiting. */
        static synchronized void attach(LocalSocket s) {
            int id = System.identityHashCode(s);
            DataOutputStream out;
            try {
                out = new DataOutputStream(s.getOutputStream());
            } catch (Throwable t) {
                // peer already gone (killed host, RST'd backlog conn)
                System.out.println("media attach #" + id + ": dead-peer drop");
                try { s.close(); } catch (Throwable ignore) {}
                return;
            }
            if (running && curSock == null) {          // stream waiting for a reader
                curSock = s;
                curOut = out;
                curWrote = false;
                System.out.println("media attach #" + id + ": promote g" + gen);
                return;
            }
            if (running) {                             // already serving one conn
                // Queue as reserve (newest waiter wins) instead of
                // busy-rejecting: if the serving conn is a backlog ghost
                // its first write fails and mediaDead promotes this
                // reserve; if the serving conn is live, the reserve
                // just waits — a stray conn must not displace a live
                // reader mid-stream.
                if (resvSock != null) {
                    try { resvSock.close(); } catch (Throwable ignore) {}
                }
                resvSock = s;
                resvOut = out;
                System.out.println("media attach #" + id + ": reserve g" + gen);
                return;
            }
            // !running — watch conn (armed watchers hold a media conn
            // for ui_event pushes, §4.9). Held in a capped set: every
            // conn gets the push, write-failure drops the corpse —
            // no conn ever displaces a live reader (t4 pendOut guard).
            if (watchConns.size() >= WATCH_CONN_CAP) {
                WatchConn old = watchConns.pollFirst();
                try { old.s.close(); } catch (Throwable ignore) {}
                System.out.println("media attach #" + id
                        + ": evict oldest watch conn (cap "
                        + WATCH_CONN_CAP + ")");
            }
            watchConns.addLast(new WatchConn(s, out));
            System.out.println("media attach #" + id + ": watch conn n="
                    + watchConns.size());
            // flush hits stashed while no live conn existed (arm-attach
            // race / all-corpse window) — the newest conn is the most
            // likely real reader.
            if (!pendingHits.isEmpty()) {
                int flushed = 0;
                for (Map<String, Object> h : pendingHits) {
                    try {
                        push(out, h);
                        flushed++;
                    } catch (Throwable ignore) {
                        break;      // dead conn — it drops on next push
                    }
                }
                pendingHits.clear();
                if (flushed > 0) {
                    System.out.println("watch attach #" + id
                            + ": flushed " + flushed + " stashed hit(s)");
                }
            }
        }

        static synchronized Map<String, Object> start(
                Session o, UiAutomation uia, int fps, float scale, int quality)
                throws Exception {
            if (running) throw new OpError("ERR_STREAM_BUSY", "stream already running");
            owner = o; running = true; frames = 0; dropped = 0; seq = 0;
            nullWarned = false; capNulls = 0; capNsSum = 0; encNsSum = 0;
            sFps = Math.max(1, Math.min(30, fps));
            sScale = scale; sQuality = quality;
            queue = new java.util.concurrent.ArrayBlockingQueue<>(1);
            // §4.8 order is start-ack -> media connect; a pre-start held
            // conn may already be dead (can't be probed without IO) —
            // drop ALL watch conns rather than promote a corpse.
            for (WatchConn c = watchConns.pollFirst(); c != null;
                    c = watchConns.pollFirst()) {
                try { c.s.close(); } catch (Throwable ignore) {}
            }
            if (resvSock != null) {   // can't be set while !running — defensive
                try { resvSock.close(); } catch (Throwable ignore) {}
                resvSock = null; resvOut = null;
            }
            final int g = ++gen;
            producer = new Thread(() -> produce(uia, g), "pd-stream-prod");
            writer = new Thread(() -> writeLoop(g), "pd-stream-wr");
            producer.start(); writer.start();
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("ok", true);
            m.put("socket", MEDIA_SOCKET);
            m.put("effective_fps_hint", sFps);
            return m;
        }

        static synchronized Map<String, Object> stop(Session caller)
                throws Exception {
            if (running && caller != null && owner != null && caller != owner)
                throw new OpError("ERR_STREAM_BUSY", "stream owned by another session");
            halt();
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("ok", true);
            m.put("frames_sent", frames);
            m.put("dropped", dropped);
            m.put("cap", "uia");                   // capture backend (ADR-008)
            m.put("cap_nulls", capNulls);        // takeScreenshot() null count
            m.put("cap_ms", frames > 0 ? capNsSum / frames / 1_000_000L : 0);
            m.put("enc_ms", frames > 0 ? encNsSum / frames / 1_000_000L : 0);
            return m;
        }

        /** control session ended -> implicit stop (session binding). */
        static synchronized void ctrlClosed(Session s) {
            if (running && s == owner) {
                try { halt(); } catch (Throwable ignore) {}
            }
        }

        /** media conn write failed: promote the queued reader if one
         *  is waiting (the dead conn may have been a backlog ghost
         *  promoted ahead of it, t4); a dead conn with no reserve
         *  still stops the stream per §4.8. */
        static synchronized void mediaDead() { mediaDead(gen); }

        static synchronized void mediaDead(int g) {
            if (!running || g != gen) {
                System.out.println("mediaDead g" + g + " ignored (running="
                        + running + " gen=" + gen + ")");
                return;
            }
            if (resvSock != null) {
                try { if (curSock != null) curSock.close(); } catch (Throwable ignore) {}
                // a reserve conn was never written to — it re-enters
                // untested, so a promoted dead reserve still counts as
                // ghost on its first failed write.
                curSock = resvSock; curOut = resvOut; curWrote = false;
                resvSock = null; resvOut = null;
                System.out.println("mediaDead g" + g + ": reserve #"
                        + System.identityHashCode(curSock) + " promoted");
                return;
            }
            if (!curWrote) {
                // conn died without a single successful write — backlog
                // ghost promoted ahead of the real reader: drop it and
                // keep waiting; the reader's attach re-promotes.
                try { if (curSock != null) curSock.close(); } catch (Throwable ignore) {}
                curSock = null; curOut = null;
                System.out.println("mediaDead g" + g + ": ghost dropped, awaiting reader");
                return;
            }
            System.out.println("mediaDead g" + g + ": no reserve, halting stream");
            try { halt(); } catch (Throwable ignore) {}
        }

        private static void halt() {
            running = false;
            if (producer != null) producer.interrupt();
            // closing the socket also unblocks a writer stuck in write()
            try { if (curSock != null) curSock.close(); } catch (Throwable ignore) {}
            curSock = null; curOut = null; curWrote = false;
            try { if (resvSock != null) resvSock.close(); } catch (Throwable ignore) {}
            resvSock = null; resvOut = null;
            joinQuiet(producer); joinQuiet(writer);
            producer = writer = null; owner = null;
        }

        private static void joinQuiet(Thread t) {
            if (t == null) return;
            try { t.join(2000); } catch (Throwable ignore) {}
        }



        private static void produce(UiAutomation uia, int myGen) {
            long intervalNs = 1_000_000_000L / sFps;
            java.util.concurrent.ArrayBlockingQueue<byte[]> q = queue;
            ByteArrayOutputStream bos = new ByteArrayOutputStream(1 << 16);
            while (myGen == gen && running) {
                long t0 = System.nanoTime();
                byte[] f;
                try {
                    Bitmap bmp = shotSync(uia);
                    if (bmp == null) {
                        capNulls++;
                        if (seq == 0 && !nullWarned) {
                            nullWarned = true;
                            System.out.println("cap: null bitmap");
                        }
                        try { Thread.sleep(50); } catch (InterruptedException e) { return; }
                        continue;
                    }
                    long t1 = System.nanoTime();
                    Bitmap outBmp = bmp;
                    if (sScale < 1f) {
                        outBmp = Bitmap.createScaledBitmap(bmp,
                                Math.max(1, Math.round(bmp.getWidth() * sScale)),
                                Math.max(1, Math.round(bmp.getHeight() * sScale)), true);
                    }
                    long t2 = System.nanoTime();
                    bos.reset();
                    outBmp.compress(Bitmap.CompressFormat.JPEG, sQuality, bos);
                    long t3 = System.nanoTime();
                    Map<String, Object> p = new LinkedHashMap<>();
                    p.put("format", "jpeg");
                    p.put("w", outBmp.getWidth());
                    p.put("h", outBmp.getHeight());
                    p.put("ts_ns", t0);
                    p.put("take_ns", t2 - t0);   // capture+scale
                    p.put("enc_ns", t3 - t2);    // jpeg encode
                    p.put("dropped", dropped);
                    p.put("data", bos.toByteArray());
                    f = frame(++seq, p);
                    capNsSum += t2 - t0;
                    encNsSum += t3 - t2;
                    outBmp.recycle();
                    if (outBmp != bmp) bmp.recycle();
                } catch (Throwable t) {
                    System.out.println("produce err: " + t);
                    continue;
                }
                if (!q.offer(f)) dropped++;              // backpressure: drop
                long rest = intervalNs - (System.nanoTime() - t0);
                if (rest > 1_000_000L) {
                    try { Thread.sleep(rest / 1_000_000L); }
                    catch (InterruptedException e) { return; }
                }
            }
        }

        private static void writeLoop(int myGen) {
            // pin this generation's queue — the static `queue` field is
            // replaced by the next start() while this thread may still
            // be unwinding.
            java.util.concurrent.ArrayBlockingQueue<byte[]> q = queue;
            while (myGen == gen && (running || !q.isEmpty())) {
                byte[] f;
                try { f = q.take(); }
                catch (InterruptedException e) { return; }
                DataOutputStream o;
                synchronized (Streamer.class) { o = curOut; }
                if (o == null) {
                    synchronized (Streamer.class) { dropped++; }
                    continue;
                }
                try {
                    o.write(f);
                    o.flush();
                    synchronized (Streamer.class) { frames++; curWrote = true; }
                } catch (Throwable t) {
                    mediaDead(myGen);
                    // reserve promoted -> running still true, keep
                    // draining to the new curSock; stale-gen failure or
                    // halt -> the loop exits on `running`/`queue` state.
                }
            }
        }

        /** ui_event push (§4.8 reserved, ch=1 type 0x0002): a one-shot
         *  WATCH_HIT — written to the active stream reader, else to
         *  EVERY held watch conn (watch clients hold them while armed).
         *  No media conn = event dropped; the action already ran.
         *
         *  t4 ghost guard: a held conn whose write fails is a corpse
         *  (dirty-killed host / backlog ghost) — dropped on the spot.
         *  Broadcasting means a corpse ahead of the real reader can no
         *  longer eat the hit, and a newly attached conn gets the event
         *  even while an older corpse is still being reaped. `wrote`
         *  distinguishes ghost-vs-reader in the log — the curWrote
         *  equivalent; the behavioural heal is the broadcast itself. */
        static synchronized void uiEvent(Map<String, Object> p) {
            if (curOut != null) {
                try {
                    push(curOut, p);
                } catch (Throwable t) {
                    try { if (curSock != null) curSock.close(); }
                    catch (Throwable ignore) {}
                    mediaDead();
                }
                return;
            }
            int delivered = 0;
            for (java.util.Iterator<WatchConn> it =
                    watchConns.iterator(); it.hasNext();) {
                WatchConn c = it.next();
                try {
                    push(c.o, p);
                    c.wrote = true;
                    delivered++;
                } catch (Throwable t) {
                    try { c.s.close(); } catch (Throwable ignore) {}
                    it.remove();
                    System.out.println("uiEvent: watch-"
                            + (c.wrote ? "reader" : "ghost") + " #"
                            + System.identityHashCode(c.s)
                            + " dropped (n=" + watchConns.size() + ")");
                }
            }
            System.out.println("uiEvent: delivered=" + delivered
                    + " live=" + watchConns.size());
            if (delivered == 0) {
                long now = System.currentTimeMillis();
                if (now - lastHitDropTs > PENDING_HIT_TTL_MS) {
                    pendingHits.clear();        // expire stale backlog
                }
                if (pendingHits.size() >= PENDING_HIT_CAP) {
                    pendingHits.pollFirst();
                }
                pendingHits.addLast(p);
                lastHitDropTs = now;
                System.out.println("uiEvent: no live watch conn — hit "
                        + "stashed (n=" + pendingHits.size() + ")");
            }
        }

        /** ch=1 type 0x0002 push frame onto `o` (§4.8 ui_event wire). */
        private static void push(DataOutputStream o, Map<String, Object> p)
                throws Exception {
            byte[] body = Msgpack.write(p);
            o.write("PD01".getBytes("US-ASCII"));
            o.writeByte(PROTO_VER);
            o.writeByte(CH_MEDIA);
            o.writeShort(T_WATCH_HIT);
            o.writeShort(0);                 // flags: complete, no MORE
            o.writeInt(0);                   // req_id unused for push
            o.writeInt(body.length);
            o.write(body);
            o.flush();
        }

        /** ch=1 FRAME frame: flags STREAM_MORE, req_id = seq (§4.8). */
        private static byte[] frame(long s, Map<String, Object> p) throws Exception {
            byte[] body = Msgpack.write(p);
            ByteArrayOutputStream b = new ByteArrayOutputStream(18 + body.length);
            DataOutputStream w = new DataOutputStream(b);
            w.write("PD01".getBytes("US-ASCII"));
            w.writeByte(PROTO_VER);
            w.writeByte(CH_MEDIA);
            w.writeShort(T_FRAME);
            w.writeShort(F_MORE);
            w.writeInt((int) s);
            w.writeInt(body.length);
            w.write(body);
            w.flush();
            return b.toByteArray();
        }
    }

    // ---------- input injection ----------------------------------------------

    /** IM-first injector with UiAutomation.injectInputEvent fallback
     *  (duchamp showed INJECT_EVENTS can be revoked per-OEM). */
    private static final class Injector {
        private final UiAutomation uia;
        private Object im;                    // InputManagerGlobal, may be null
        private Method inject;                // injectInputEvent(InputEvent,int)
        private Method uiaInject;             // injectInputEvent(InputEvent,boolean)
        private int imFailures;

        Injector(UiAutomation uia) {
            this.uia = uia;
            for (String n : new String[]{
                    "android.hardware.input.InputManagerGlobal",
                    "android.hardware.input.InputManager"}) {
                try {
                    Class<?> c = Class.forName(n);
                    Method g = c.getDeclaredMethod("getInstance");
                    g.setAccessible(true);
                    this.im = g.invoke(null);
                    break;
                } catch (Throwable ignore) {
                }
            }
            if (im != null) {
                try {
                    this.inject = im.getClass().getDeclaredMethod("injectInputEvent",
                            InputEvent.class, int.class);
                    inject.setAccessible(true);
                } catch (Throwable t) {
                    im = null;
                }
            }
            try {
                this.uiaInject = UiAutomation.class.getDeclaredMethod(
                        "injectInputEvent", InputEvent.class, boolean.class);
                uiaInject.setAccessible(true);
            } catch (Throwable ignore) {
                this.uiaInject = null;
            }
        }

        String backend() {
            if (im != null && imFailures == 0) return "im";
            if (uiaInject != null && (im == null || imFailures > 0)) return "uia";
            if (im != null) return "im+uia-fallback";
            return "none";
        }

        /** Is the named backend channel available right now? */
        boolean has(String b) {
            return "im".equals(b) ? im != null : uiaInject != null;
        }

        private static void recycleEv(InputEvent ev) {
            // KeyEvent has no public recycle on this level; MotionEvent does.
            if (ev instanceof MotionEvent) ((MotionEvent) ev).recycle();
        }

        /** @param force null=fallback chain, "im"/"uia"=that channel only. */
        private boolean put(InputEvent ev, int mode, String force) {
            if (!"uia".equals(force) && im != null && imFailures < 3) {
                try {
                    Boolean r = (Boolean) inject.invoke(im, ev, mode);
                    if (Boolean.TRUE.equals(r)) {
                        recycleEv(ev);
                        return true;
                    }
                } catch (Throwable t) {
                    imFailures++;
                }
                if (force != null) {           // forced im failed: no fallback
                    recycleEv(ev);
                    return false;
                }
            }
            if (!"im".equals(force)) {
                // fallback: UiAutomation.injectInputEvent (slower but OEM-safe)
                try {
                    if (uiaInject != null) {
                        Boolean r;
                        synchronized (uia) {
                            r = (Boolean) uiaInject.invoke(uia, ev, mode == 2);
                        }
                        if (Boolean.TRUE.equals(r)) {
                            if (force == null) imFailures++; // prefer IM, tolerate flakiness
                            recycleEv(ev);
                            return true;
                        }
                    }
                } catch (Throwable t) {
                    imFailures++;
                }
            }
            recycleEv(ev);
            return false;
        }

        /** Generic MotionEvent point sequence: [{action,x,y,t_ms,pid?}].
         *  `pid` identifies a finger; >1 distinct pid takes the
         *  multi-pointer path (Android MotionEvent carries the WHOLE active
         *  pointer set per event — we track pid->coords state and emit
         *  ACTION_DOWN / _POINTER_DOWN(idx) / MOVE / _POINTER_UP(idx) / UP).
         *  Single-pid lists keep the legacy one-event-per-point path. */
        int points(List<?> pts, boolean waitFinish, String force) {
            long t0 = SystemClock.uptimeMillis();
            int n = pts.size();
            // multi-pointer? scan distinct pids
            java.util.LinkedHashSet<Object> pids = new java.util.LinkedHashSet<>();
            for (Object o : pts) {
                if (o instanceof Map) pids.add(num(((Map<?, ?>) o).get("pid"), 0));
            }
            if (pids.size() > 1) return multiPoints(pts, waitFinish, force, t0);
            int ok = 0;
            for (int i = 0; i < n; i++) {
                Map<?, ?> p = pts.get(i) instanceof Map
                        ? (Map<?, ?>) pts.get(i) : new LinkedHashMap<>();
                int action = (int) num(p.get("action"), 0);
                MotionEvent ev = MotionEvent.obtain(t0,
                        t0 + (long) num(p.get("t_ms"), 0),
                        action,
                        (float) num(p.get("x"), 0), (float) num(p.get("y"), 0), 0);
                ev.setSource(InputDevice.SOURCE_TOUCHSCREEN);
                if (put(ev, waitFinish && i == n - 1 ? 2 : 0, force)) ok++;
            }
            return ok;
        }

        /** Multi-pointer path: every emitted MotionEvent carries all
         *  currently-down pointers (insertion-ordered index = the wire's
         *  pid order of appearance). */
        private int multiPoints(List<?> pts, boolean waitFinish, String force,
                                long t0) {
            int ok = 0, n = pts.size();
            // active pointer state: insertion-ordered pid -> coords
            java.util.LinkedHashMap<Long, float[]> active = new java.util.LinkedHashMap<>();
            for (int i = 0; i < n; i++) {
                Map<?, ?> p = pts.get(i) instanceof Map
                        ? (Map<?, ?>) pts.get(i) : new LinkedHashMap<>();
                int action = (int) num(p.get("action"), 0);
                long pid = (long) num(p.get("pid"), 0);
                float x = (float) num(p.get("x"), 0);
                float y = (float) num(p.get("y"), 0);
                int code;
                if (action == MotionEvent.ACTION_DOWN) {
                    active.put(pid, new float[]{x, y});
                    if (active.size() == 1) {
                        code = MotionEvent.ACTION_DOWN;
                    } else {
                        int idx = indexOf(active, pid);
                        code = MotionEvent.ACTION_POINTER_DOWN
                                | (idx << MotionEvent.ACTION_POINTER_INDEX_SHIFT);
                    }
                } else if (action == MotionEvent.ACTION_MOVE) {
                    float[] c = active.get(pid);
                    if (c != null) { c[0] = x; c[1] = y; }
                    code = MotionEvent.ACTION_MOVE;
                } else { // UP — event still carries the departing pointer
                    if (active.size() <= 1) {
                        code = MotionEvent.ACTION_UP;
                    } else {
                        int idx = indexOf(active, pid);
                        code = MotionEvent.ACTION_POINTER_UP
                                | (idx << MotionEvent.ACTION_POINTER_INDEX_SHIFT);
                    }
                }
                if (active.isEmpty()) continue; // UP/MOVE with no active
                                                 // pointer: obtain(…,cnt=0)
                                                 // would crash INTERNAL
                MotionEvent ev = obtainMulti(t0,
                        t0 + (long) num(p.get("t_ms"), 0), code, active);
                ev.setSource(InputDevice.SOURCE_TOUCHSCREEN);
                if (put(ev, waitFinish && i == n - 1 ? 2 : 0, force)) ok++;
                if (action == MotionEvent.ACTION_UP) active.remove(pid);
            }
            return ok;
        }

        private static int indexOf(java.util.LinkedHashMap<Long, float[]> m,
                                   long pid) {
            int i = 0;
            for (Long k : m.keySet()) { if (k == pid) return i; i++; }
            return m.size() - 1;
        }

        private static MotionEvent obtainMulti(long down, long evTime,
                                               int action,
                                               java.util.LinkedHashMap<Long, float[]> active) {
            int cnt = active.size();
            MotionEvent.PointerProperties[] props =
                    new MotionEvent.PointerProperties[cnt];
            MotionEvent.PointerCoords[] coords =
                    new MotionEvent.PointerCoords[cnt];
            int i = 0;
            for (java.util.Map.Entry<Long, float[]> e : active.entrySet()) {
                MotionEvent.PointerProperties pr =
                        new MotionEvent.PointerProperties();
                pr.id = e.getKey().intValue();
                pr.toolType = MotionEvent.TOOL_TYPE_FINGER;
                props[i] = pr;
                MotionEvent.PointerCoords pc = new MotionEvent.PointerCoords();
                pc.x = e.getValue()[0];
                pc.y = e.getValue()[1];
                pc.pressure = 1.0f;
                pc.size = 1.0f;
                coords[i] = pc;
                i++;
            }
            return MotionEvent.obtain(down, evTime, action, cnt, props, coords,
                    0, 0, 1.0f, 1.0f, 0, 0,
                    InputDevice.SOURCE_TOUCHSCREEN, 0);
        }

        /** kind="Key": KeyEvent DOWN+UP over the same injectInputEvent. */
        int key(int keycode, int meta, boolean waitFinish, String force) {
            long now = SystemClock.uptimeMillis();
            int ok = 0;
            for (int a : new int[]{KeyEvent.ACTION_DOWN, KeyEvent.ACTION_UP}) {
                KeyEvent ev = new KeyEvent(now, now, a, keycode, 0, meta,
                        -1, 0, 0, InputDevice.SOURCE_KEYBOARD);
                if (put(ev, waitFinish && a == KeyEvent.ACTION_UP ? 2 : 0,
                        force)) ok++;
            }
            return ok;
        }

        int tap(float x, float y, boolean waitFinish, String force) {
            long now = SystemClock.uptimeMillis();
            int ok = 0;
            for (int a : new int[]{MotionEvent.ACTION_DOWN, MotionEvent.ACTION_UP}) {
                MotionEvent ev = MotionEvent.obtain(now, now, a, x, y, 0);
                ev.setSource(InputDevice.SOURCE_TOUCHSCREEN);
                if (put(ev, waitFinish ? 2 : 0, force)) ok++;
            }
            return ok;
        }

        int swipe(float x1, float y1, float x2, float y2, int durMs, int steps,
                  boolean waitFinish, String force) {
            long t0 = SystemClock.uptimeMillis();
            int ok = 0;
            for (int i = 0; i <= steps; i++) {
                int action = i == 0 ? MotionEvent.ACTION_DOWN
                        : i == steps ? MotionEvent.ACTION_UP : MotionEvent.ACTION_MOVE;
                float f = (float) i / steps;
                MotionEvent ev = MotionEvent.obtain(t0, t0 + (long) (durMs * f),
                        action, x1 + (x2 - x1) * f, y1 + (y2 - y1) * f, 0);
                ev.setSource(InputDevice.SOURCE_TOUCHSCREEN);
                int mode = (waitFinish && i == steps) ? 2 : 0;
                if (put(ev, mode, force)) ok++;
            }
            return ok;
        }
    }

    // ---------- UI tree: dump + find ------------------------------------------

    private static final class Dumper {
        private final UiAutomation uia;

        Dumper(UiAutomation uia) { this.uia = uia; }

        Rect displayBounds() {
            synchronized (uia) {
            AccessibilityNodeInfo root = uia.getRootInActiveWindow();
            if (root == null) return new Rect();
            Rect r = new Rect();
            root.getBoundsInScreen(r);
            root.recycle();
            return r;
            }
        }

        /** DFS for the nth node matching the selector (fresh tree per call). */
        AccessibilityNodeInfo find(Selector sel) {
            synchronized (uia) {
            AccessibilityNodeInfo root = uia.getRootInActiveWindow();
            if (root == null) return null;
            int[] seen = {0};
            AccessibilityNodeInfo hit = dfs(root, sel, seen);
            root.recycle();
            return hit;
            }
        }

        private AccessibilityNodeInfo dfs(AccessibilityNodeInfo n, Selector sel,
                                          int[] seen) {
            if (sel.match(n) && seen[0]++ == sel.index) {
                return AccessibilityNodeInfo.obtain(n);
            }
            int kids = n.getChildCount();
            for (int i = 0; i < kids; i++) {
                AccessibilityNodeInfo ch = n.getChild(i);
                if (ch != null) {
                    AccessibilityNodeInfo hit = dfs(ch, sel, seen);
                    ch.recycle();
                    if (hit != null) return hit;
                }
            }
            return null;
        }

        /** The currently focused editable node, or null. */
        AccessibilityNodeInfo focusedEditable() {
            synchronized (uia) {
            AccessibilityNodeInfo root = uia.getRootInActiveWindow();
            if (root == null) return null;
            AccessibilityNodeInfo hit = findFocused(root);
            root.recycle();
            return hit;
            }
        }

        private AccessibilityNodeInfo findFocused(AccessibilityNodeInfo n) {
            if (n.isEditable() && n.isFocused()) return AccessibilityNodeInfo.obtain(n);
            int kids = n.getChildCount();
            for (int i = 0; i < kids; i++) {
                AccessibilityNodeInfo ch = n.getChild(i);
                if (ch != null) {
                    AccessibilityNodeInfo hit = findFocused(ch);
                    ch.recycle();
                    if (hit != null) return hit;
                }
            }
            return null;
        }

        /** Structured node info for QUERY/wait_until results. */
        static Map<String, Object> nodeInfo(AccessibilityNodeInfo n) {
            return nodeInfo(n, null);
        }

        /** @param attrs when non-null, prune the result to just these keys. */
        static Map<String, Object> nodeInfo(AccessibilityNodeInfo n, Set<String> attrs) {
            Rect b = new Rect();
            n.getBoundsInScreen(b);
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("text", n.getText() == null ? "" : n.getText().toString());
            m.put("id", n.getViewIdResourceName() == null ? ""
                    : n.getViewIdResourceName());
            m.put("desc", n.getContentDescription() == null ? ""
                    : n.getContentDescription().toString());
            List<Object> bounds = new ArrayList<>();
            bounds.add(b.left); bounds.add(b.top); bounds.add(b.right); bounds.add(b.bottom);
            m.put("bounds", bounds);
            m.put("class", n.getClassName() == null ? "" : n.getClassName().toString());
            m.put("scrollable", n.isScrollable());
            m.put("clickable", n.isClickable());
            m.put("enabled", n.isEnabled());
            if (attrs != null) m.keySet().retainAll(attrs);
            return m;
        }

        /** u2-compatible hierarchy XML. */
        Map<String, Object> xml() {
            long t0 = System.nanoTime();
            synchronized (uia) {
            AccessibilityNodeInfo root = uia.getRootInActiveWindow();
            StringBuilder sb = new StringBuilder(64 * 1024);
            sb.append("<?xml version='1.0' encoding='UTF-8' standalone='yes' ?>");
            int[] count = {0};
            if (root != null) {
                sb.append("<hierarchy rotation=\"0\">");
                write(root, sb, 0, count);
                sb.append("</hierarchy>");
                root.recycle();
            }
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("format", "xml");
            m.put("nodes", count[0]);
            m.put("cost_ns", System.nanoTime() - t0);
            m.put("xml", sb.toString()); // wire contract field
            return m;
            }
        }

        private void write(AccessibilityNodeInfo n, StringBuilder sb, int idx, int[] count) {
            if (count[0] > 4096) return; // pathological trees
            count[0]++;
            Rect b = new Rect();
            n.getBoundsInScreen(b);
            sb.append("<node index=\"").append(idx).append('"')
                    .append(" text=\"").append(esc(n.getText())).append('"')
                    .append(" resource-id=\"").append(esc(n.getViewIdResourceName())).append('"')
                    .append(" class=\"").append(esc(n.getClassName())).append('"')
                    .append(" package=\"").append(esc(n.getPackageName())).append('"')
                    .append(" content-desc=\"").append(esc(n.getContentDescription())).append('"')
                    .append(" checkable=\"").append(n.isCheckable()).append('"')
                    .append(" checked=\"").append(n.isChecked()).append('"')
                    .append(" clickable=\"").append(n.isClickable()).append('"')
                    .append(" enabled=\"").append(n.isEnabled()).append('"')
                    .append(" focusable=\"").append(n.isFocusable()).append('"')
                    .append(" focused=\"").append(n.isFocused()).append('"')
                    .append(" scrollable=\"").append(n.isScrollable()).append('"')
                    .append(" long-clickable=\"").append(n.isLongClickable()).append('"')
                    .append(" password=\"").append(n.isPassword()).append('"')
                    .append(" selected=\"").append(n.isSelected()).append('"')
                    .append(" bounds=\"").append('[').append(b.left).append(',').append(b.top)
                    .append("][").append(b.right).append(',').append(b.bottom).append(']').append('"');
            int kids = n.getChildCount();
            if (kids == 0) {
                sb.append("/>");
                return;
            }
            sb.append('>');
            for (int i = 0; i < kids; i++) {
                AccessibilityNodeInfo ch = n.getChild(i);
                if (ch != null) {
                    write(ch, sb, i, count);
                    ch.recycle();
                }
            }
            sb.append("</node>");
        }

        private static String esc(CharSequence s) {
            if (s == null) return "";
            String v = s.toString();
            StringBuilder b = new StringBuilder(v.length());
            for (int i = 0; i < v.length(); i++) {
                char c = v.charAt(i);
                switch (c) {
                    case '&': b.append("&amp;"); break;
                    case '<': b.append("&lt;"); break;
                    case '>': b.append("&gt;"); break;
                    case '"': b.append("&quot;"); break;
                    case '\'': b.append("&apos;"); break;
                    default: b.append(c < 0x20 && c != '\n' && c != '\t' ? ' ' : c);
                }
            }
            return b.toString();
        }
    }

    // ---------- minimal msgpack codec (map/array/str/bin/int/float/bool/nil) ---

    static final class Msgpack {
        static Object read(byte[] d) {
            return new Reader(d).val();
        }

        static byte[] write(Object o) throws Exception {
            ByteArrayOutputStream bos = new ByteArrayOutputStream();
            writeTo(bos, o);
            return bos.toByteArray();
        }

        private static void writeTo(ByteArrayOutputStream o, Object v) throws Exception {
            if (v == null) { o.write(0xc0); return; }
            if (v instanceof Boolean) { o.write((Boolean) v ? 0xc3 : 0xc2); return; }
            if (v instanceof Number) {
                Number n = (Number) v;
                if (v instanceof Float || v instanceof Double) {
                    o.write(0xcb);
                    long bits = Double.doubleToLongBits(n.doubleValue());
                    for (int i = 7; i >= 0; i--) o.write((int) (bits >>> (8 * i)) & 0xff);
                    return;
                }
                long x = n.longValue();
                if (x >= 0 && x < 128) { o.write((int) x); return; }
                if (x >= -32 && x < 0) { o.write(0xe0 | (int) (x & 0x1f)); return; }
                if (x >= 0 && x <= 0xff) { o.write(0xcc); o.write((int) x); return; }
                if (x >= 0 && x <= 0xffff) { o.write(0xcd); o.write((int) (x >> 8)); o.write((int) x); return; }
                if (x >= 0 && x <= 0xffffffffL) {
                    o.write(0xce);
                    for (int i = 3; i >= 0; i--) o.write((int) (x >>> (8 * i)) & 0xff);
                    return;
                }
                o.write(0xd3);
                for (int i = 7; i >= 0; i--) o.write((int) (x >>> (8 * i)) & 0xff);
                return;
            }
            if (v instanceof byte[]) {
                byte[] b = (byte[]) v;
                if (b.length < 256) { o.write(0xc4); o.write(b.length); }
                else if (b.length < 65536) { o.write(0xc5); o.write(b.length >> 8); o.write(b.length); }
                else { o.write(0xc6); i32(o, b.length); }
                o.write(b, 0, b.length);
                return;
            }
            if (v instanceof String || v instanceof CharSequence) {
                byte[] b = v.toString().getBytes("UTF-8");
                if (b.length < 32) { o.write(0xa0 | b.length); }
                else if (b.length < 256) { o.write(0xd9); o.write(b.length); }
                else if (b.length < 65536) { o.write(0xda); o.write(b.length >> 8); o.write(b.length); }
                else { o.write(0xdb); i32(o, b.length); }
                o.write(b, 0, b.length);
                return;
            }
            if (v instanceof List) {
                List<?> l = (List<?>) v;
                if (l.size() < 16) o.write(0x90 | l.size());
                else { o.write(0xdc); o.write(l.size() >> 8); o.write(l.size()); }
                for (Object e : l) writeTo(o, e);
                return;
            }
            if (v instanceof Map) {
                Map<?, ?> m = (Map<?, ?>) v;
                if (m.size() < 16) o.write(0x80 | m.size());
                else { o.write(0xde); o.write(m.size() >> 8); o.write(m.size()); }
                for (Map.Entry<?, ?> e : m.entrySet()) {
                    writeTo(o, String.valueOf(e.getKey()));
                    writeTo(o, e.getValue());
                }
                return;
            }
            writeTo(o, String.valueOf(v));
        }

        private static void i32(ByteArrayOutputStream o, int x) {
            for (int i = 3; i >= 0; i--) o.write((x >>> (8 * i)) & 0xff);
        }

        private static final class Reader {
            private final byte[] d;
            private int p;

            Reader(byte[] d) { this.d = d; }

            Object val() {
                if (p >= d.length) return null;
                int b = d[p++] & 0xff;
                if (b <= 0x7f) return b;                       // +fixint
                if (b >= 0xe0) return b - 256;                 // -fixint
                if (b >= 0xa0 && b <= 0xbf) return str(b & 0x1f);
                if (b >= 0x90 && b <= 0x9f) return arr(b & 0x0f);
                if (b >= 0x80 && b <= 0x8f) return map(b & 0x0f);
                switch (b) {
                    case 0xc0: return null;
                    case 0xc2: return false;
                    case 0xc3: return true;
                    case 0xc4: return bin(u8());
                    case 0xc5: return bin(u16());
                    case 0xc6: return bin(i32());
                    case 0xca:
                        return Float.intBitsToFloat(i32());
                    case 0xcb:
                        return Double.longBitsToDouble(i64());
                    case 0xcc: return u8();
                    case 0xcd: return u16();
                    case 0xce: return i32() & 0xffffffffL;
                    case 0xcf: return i64();
                    case 0xd0: return (byte) u8();
                    case 0xd1: return (short) u16();
                    case 0xd2: return i32();
                    case 0xd3: return i64();
                    case 0xd9: return str(u8());
                    case 0xda: return str(u16());
                    case 0xdb: return str(i32());
                    case 0xdc: return arr(u16());
                    case 0xdd: return arr(i32());
                    case 0xde: return map(u16());
                    case 0xdf: return map(i32());
                    default: throw new IllegalArgumentException("msgpack 0x" + Integer.toHexString(b));
                }
            }

            private int u8() { return d[p++] & 0xff; }
            private int u16() { return (u8() << 8) | u8(); }
            private int i32() { return (u8() << 24) | (u8() << 16) | (u8() << 8) | u8(); }
            private long i64() { return ((long) i32() << 32) | (i32() & 0xffffffffL); }

            private Object str(int n) {
                String s = new String(d, p, n, java.nio.charset.StandardCharsets.UTF_8);
                p += n;
                return s;
            }

            private Object bin(int n) {
                byte[] b = new byte[n];
                System.arraycopy(d, p, b, 0, n);
                p += n;
                return b;
            }

            private Object arr(int n) {
                List<Object> l = new ArrayList<>(n);
                for (int i = 0; i < n; i++) l.add(val());
                return l;
            }

            private Object map(int n) {
                Map<String, Object> m = new LinkedHashMap<>();
                for (int i = 0; i < n; i++) m.put(String.valueOf(val()), val());
                return m;
            }
        }
    }

    // ---------- tiny helpers ---------------------------------------------------

    /** ERROR-flagged payload — wire contract is {code,message}; keep `error`
     *  alias for the python dev client. */
    private static Map<String, Object> errPayload(String code, String message) {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("code", code);
        m.put("message", message);
        m.put("error", message);
        return m;
    }

    private static Map<String, Object> map(Object... kv) {
        Map<String, Object> m = new LinkedHashMap<>();
        for (int i = 0; i + 1 < kv.length; i += 2) m.put(String.valueOf(kv[i]), kv[i + 1]);
        return m;
    }

    private static String str(Object o, String def) {
        return o == null ? def : String.valueOf(o);
    }

    private static double num(Object o, double def) {
        return o instanceof Number ? ((Number) o).doubleValue() : def;
    }

    private static boolean bool(Object o, boolean def) {
        return o instanceof Boolean ? (Boolean) o : def;
    }
}
