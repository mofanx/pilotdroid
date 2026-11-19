import android.app.UiAutomation;
import android.graphics.Bitmap;
import android.graphics.Rect;
import android.hardware.input.InputManager;
import android.net.LocalServerSocket;
import android.net.LocalSocket;
import android.os.Looper;
import android.os.SystemClock;
import android.view.InputDevice;
import android.view.InputEvent;
import android.view.MotionEvent;
import android.view.SurfaceControl;
import android.view.accessibility.AccessibilityNodeInfo;

import java.io.InputStream;
import java.io.OutputStream;
import java.lang.reflect.Constructor;
import java.lang.reflect.Method;
import java.util.Locale;

/**
 * Spike-2: verify ALL agent capabilities inside ONE plain app_process process.
 *
 *   CLASSPATH=/data/local/tmp/spike2.jar app_process /data/local/tmp SpikeAgent <cmd>
 *
 * cmds: all | connect | dump | inject | shot | sock | heartbeat
 * No uiautomator runtest, no uiautomator.jar on classpath.
 */
public final class SpikeAgent {
    private static final int FLAG_DONT_SUPPRESS = 1; // UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES
    private static UiAutomation uia;

    public static void main(String[] args) throws Exception {
        String cmd = args.length > 0 ? args[0] : "all";
        float x = args.length > 1 && !cmd.equals("heartbeat")
                ? Float.parseFloat(args[1]) : 700f;
        float y = args.length > 2 ? Float.parseFloat(args[2]) : 1600f;
        switch (cmd) {
            case "all":
                connect();
                testDump();
                testInjectUia(x, y);
                testInjectIm(x, y);
                testScreenshot();
                testSurfaceControlShot();
                break;
            case "connect":
                connect();
                System.out.println("connected");
                break;
            case "dump":
                connect();
                testDump();
                break;
            case "inject":
                connect();
                testInjectUia(x, y);
                testInjectIm(x, y);
                break;
            case "shot":
                connect();
                testScreenshot();
                testSurfaceControlShot();
                break;
            case "swipe":
                testSwipe(x, y);
                break;
            case "sock":
                testLocalAbstract();
                break;
            case "heartbeat":
                heartbeat(args.length > 1 ? args[1] : "/data/local/tmp/hb.log");
                break;
            default:
                System.out.println("unknown cmd " + cmd);
        }
    }

    // ---- capability 1: UiAutomation under plain app_process ----------------
    private static void connect() throws Exception {
        long t0 = System.nanoTime();
        try {
            Looper.prepareMainLooper();
        } catch (Throwable ignore) {
            // already prepared
        }
        // new UiAutomation(Looper, IUiAutomationConnection) — hidden ctor
        Class<?> connCls = Class.forName("android.app.UiAutomationConnection");
        Object conn = connCls.getDeclaredConstructor().newInstance();
        Class<?> connIface = Class.forName("android.app.IUiAutomationConnection");
        Constructor<UiAutomation> ctor = UiAutomation.class.getDeclaredConstructor(
                Looper.class, connIface);
        ctor.setAccessible(true);
        uia = ctor.newInstance(Looper.getMainLooper(), conn);
        Method connect = UiAutomation.class.getDeclaredMethod("connect", int.class);
        connect.setAccessible(true);
        Method getFlags = UiAutomation.class.getDeclaredMethod("getFlags");
        getFlags.setAccessible(true);
        long t1 = System.nanoTime();
        try {
            connect.invoke(uia, FLAG_DONT_SUPPRESS);
        } catch (Throwable t) {
            System.out.println("connect_flag1_fail=" + t + " retrying flag0");
            uia = ctor.newInstance(Looper.getMainLooper(),
                    connCls.getDeclaredConstructor().newInstance());
            connect.invoke(uia, 0);
        }
        long t2 = System.nanoTime();
        System.out.println(String.format(Locale.US,
                "ctor_ms=%.1f connect_ms=%.1f flags=%d",
                (t1 - t0) / 1e6, (t2 - t1) / 1e6, getFlags.invoke(uia)));
    }

    // ---- capability 2: resident UI tree dump -------------------------------
    private static void testDump() {
        int n = 30;
        long[] ts = new long[n];
        int nodes = 0;
        for (int i = 0; i < n; i++) {
            long s = System.nanoTime();
            AccessibilityNodeInfo root = uia.getRootInActiveWindow();
            nodes = root == null ? 0 : countNodes(root);
            ts[i] = System.nanoTime() - s;
            if (root != null) root.recycle();
            if (i == 0) System.out.println("dump_first_ms=" + ts[0] / 1e6);
        }
        System.out.println("dump_nodes=" + nodes);
        System.out.println("dump_stats_ms=" + stats(ts));
    }

    private static int countNodes(AccessibilityNodeInfo r) {
        int c = 1;
        for (int i = 0; i < r.getChildCount(); i++) {
            AccessibilityNodeInfo ch = r.getChild(i);
            if (ch != null) {
                c += countNodes(ch);
                ch.recycle();
            }
        }
        return c;
    }

    // ---- capability 3a: UiAutomation.injectInputEvent ----------------------
    private static void testInjectUia(float x, float y) throws Exception {
        Method inject = UiAutomation.class.getMethod("injectInputEvent", InputEvent.class,
                boolean.class);
        int n = 30;
        long[] ts = new long[n];
        int ok = 0;
        for (int i = 0; i < n; i++) {
            long s = System.nanoTime();
            Boolean r = (Boolean) injectTap(ev -> inject.invoke(uia, ev, true), x, y);
            ts[i] = System.nanoTime() - s;
            if (Boolean.TRUE.equals(r)) ok++;
        }
        System.out.println("uia_inject_ok=" + ok + "/" + n);
        System.out.println("uia_inject_stats_ms=" + stats(ts));
    }

    // ---- capability 3b: InputManagerGlobal in SAME process -----------------
    private static void testInjectIm(float x, float y) throws Exception {
        Object im = null;
        for (String name : new String[]{
                "android.hardware.input.InputManagerGlobal",
                "android.hardware.input.InputManager"}) {
            try {
                Class<?> c = Class.forName(name);
                Method m = c.getDeclaredMethod("getInstance");
                m.setAccessible(true);
                im = m.invoke(null);
                break;
            } catch (Throwable ignore) {
            }
        }
        if (im == null) {
            System.out.println("im_inject_ok=NO_INPUTMANAGER");
            return;
        }
        Method inject = im.getClass().getDeclaredMethod("injectInputEvent",
                InputEvent.class, int.class);
        inject.setAccessible(true);
        final Object fim = im;
        int n = 30;
        long[] ts = new long[n];
        int ok = 0;
        for (int i = 0; i < n; i++) {
            long s = System.nanoTime();
            Boolean r = (Boolean) injectTap(ev -> inject.invoke(fim, ev, 2), x, y);
            ts[i] = System.nanoTime() - s;
            if (Boolean.TRUE.equals(r)) ok++;
        }
        System.out.println("im_inject_ok=" + ok + "/" + n);
        System.out.println("im_inject_stats_ms=" + stats(ts));
    }

    private interface Injector {
        Object inject(MotionEvent ev) throws Exception;
    }

    private static Object injectTap(Injector ij, float x, float y) throws Exception {
        long now = SystemClock.uptimeMillis();
        Object r = null;
        for (int action : new int[]{MotionEvent.ACTION_DOWN, MotionEvent.ACTION_UP}) {
            MotionEvent ev = MotionEvent.obtain(now, now, action, x, y, 0);
            ev.setSource(InputDevice.SOURCE_TOUCHSCREEN);
            r = ij.inject(ev);
            ev.recycle();
        }
        return r;
    }

    // ---- gesture injection: swipe via MotionEvent sequence (IM path) -------
    private static void testSwipe(float x, float y) throws Exception {
        Object im = null;
        for (String name : new String[]{
                "android.hardware.input.InputManagerGlobal",
                "android.hardware.input.InputManager"}) {
            try {
                Class<?> c = Class.forName(name);
                Method m = c.getDeclaredMethod("getInstance");
                m.setAccessible(true);
                im = m.invoke(null);
                break;
            } catch (Throwable ignore) {
            }
        }
        Method inject = im.getClass().getDeclaredMethod("injectInputEvent",
                InputEvent.class, int.class);
        inject.setAccessible(true);
        int steps = 10;
        long t0 = SystemClock.uptimeMillis();
        int ok = 0;
        for (int i = 0; i <= steps; i++) {
            int action = i == 0 ? MotionEvent.ACTION_DOWN
                    : i == steps ? MotionEvent.ACTION_UP : MotionEvent.ACTION_MOVE;
            MotionEvent ev = MotionEvent.obtain(t0, t0 + i * 16, action,
                    x, y - i * 40f, 0);
            ev.setSource(InputDevice.SOURCE_TOUCHSCREEN);
            if (Boolean.TRUE.equals(inject.invoke(im, ev, 0))) ok++;
            ev.recycle();
        }
        System.out.println("swipe_events_ok=" + ok + "/" + (steps + 1)
                + " total_ms=" + (SystemClock.uptimeMillis() - t0));
    }

    // ---- capability 4a: UiAutomation.takeScreenshot ------------------------
    private static void testScreenshot() {
        try {
            int n = 10;
            long[] ts = new long[n];
            Bitmap bmp = null;
            for (int i = 0; i < n; i++) {
                long s = System.nanoTime();
                bmp = uia.takeScreenshot();
                ts[i] = System.nanoTime() - s;
            }
            if (bmp == null) {
                System.out.println("uia_shot=null");
                return;
            }
            System.out.println("uia_shot=" + bmp.getWidth() + "x" + bmp.getHeight());
            System.out.println("uia_shot_stats_ms=" + stats(ts));
            bmp.recycle();
        } catch (Throwable t) {
            System.out.println("uia_shot_fail=" + t);
        }
    }

    // ---- capability 4b: SurfaceControl hidden screenshot -------------------
    private static void testSurfaceControlShot() {
        // try known signatures across API levels
        String[][] sigs = {
                {"screenshot", "android.graphics.Rect", "int"},
                {"captureDisplay", "android.hardware.display.DisplayCaptureArgs"},
        };
        for (String[] sig : sigs) {
            try {
                if (sig[0].equals("screenshot")) {
                    Method m = SurfaceControl.class.getDeclaredMethod("screenshot",
                            Rect.class, int.class);
                    m.setAccessible(true);
                    long s = System.nanoTime();
                    Object r = m.invoke(null, new Rect(), 0);
                    long dt = System.nanoTime() - s;
                    System.out.println("sc_shot=" + (r == null ? "null" : r.getClass().getSimpleName())
                            + String.format(Locale.US, " ms=%.1f", dt / 1e6));
                } else {
                    Class.forName("android.hardware.display.DisplayCaptureArgs");
                    System.out.println("sc_captureDisplay=present(args-class-found)");
                }
                return;
            } catch (Throwable t) {
                System.out.println("sc_" + sig[0] + "_fail=" + t.getClass().getSimpleName()
                        + ": " + t.getMessage());
            }
        }
    }

    // ---- capability 5: localabstract server + adb forward loopback ---------
    private static void testLocalAbstract() throws Exception {
        LocalServerSocket srv = new LocalServerSocket("pilotdroid");
        System.out.println("localabstract_listening=pilotdroid");
        LocalSocket s = srv.accept(); // blocks until host connects via adb forward
        InputStream in = s.getInputStream();
        OutputStream out = s.getOutputStream();
        byte[] buf = new byte[64];
        int r = in.read(buf);
        out.write(buf, 0, r);
        out.flush();
        System.out.println("echo_bytes=" + r);
        s.close();
        srv.close();
    }

    // ---- daemon survival probe ---------------------------------------------
    private static void heartbeat(String path) throws Exception {
        java.io.FileWriter w = new java.io.FileWriter(path, true);
        for (int i = 0; i < 600; i++) {
            w.write("hb " + i + " " + System.currentTimeMillis() + "\n");
            w.flush();
            Thread.sleep(1000);
        }
        w.close();
    }

    private static String stats(long[] ns) {
        java.util.Arrays.sort(ns);
        double mean = 0;
        for (long v : ns) mean += v;
        mean /= ns.length;
        return String.format(Locale.US,
                "mean=%.2f p50=%.2f p95=%.2f min=%.2f max=%.2f",
                mean / 1e6, ns[ns.length / 2] / 1e6,
                ns[(int) (ns.length * 0.95)] / 1e6, ns[0] / 1e6, ns[ns.length - 1] / 1e6);
    }
}
