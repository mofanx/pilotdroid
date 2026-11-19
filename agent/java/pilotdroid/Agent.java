package pilotdroid;

import android.hardware.input.InputManager;
import android.net.LocalServerSocket;
import android.net.LocalSocket;
import android.os.SystemClock;
import android.view.InputEvent;
import android.view.MotionEvent;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStreamReader;
import java.io.OutputStream;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;

/**
 * pilotdroid device agent — skeleton for HELLO + INJECT only.
 *
 * <p>Design: docs/01-architecture.md, docs/03-device-agent.md.
 * Launch: {@code CLASSPATH=/data/local/tmp/pilotdroid/agent.jar setsid \
 *         app_process /data/local/tmp/pilotdroid pilotdroid.Agent serve}
 *
 * <p>This is a PLACEHOLDER transport: line-based commands over
 * localabstract:pilotdroid, until the binary framing protocol
 * (docs/02-protocol.md) lands in M2.
 *
 * <p>Proven on-device (spike/RESULTS.md §4, Mi 11 / Android 14):
 * InputManagerGlobal.injectInputEvent → 30/30 ok, p50 ≈ 3 ms.
 */
public final class Agent {
    private static final String SOCKET_NAME = "pilotdroid";

    private Agent() {}

    public static void main(String[] args) throws Exception {
        Injector injector = new Injector();
        if (args.length >= 1 && args[0].equals("inject") && args.length >= 3) {
            // one-shot: app_process ... Agent inject <x> <y>
            float x = Float.parseFloat(args[1]);
            float y = Float.parseFloat(args[2]);
            long t = System.nanoTime();
            boolean ok = injector.tap(x, y);
            System.out.println("inject_ok=" + ok + " ms=" + (System.nanoTime() - t) / 1e6);
            return;
        }
        serve(injector);
    }

    /** Line-protocol server: one client at a time (see docs/03 §5 concurrency). */
    private static void serve(Injector injector) throws IOException {
        LocalServerSocket server = new LocalServerSocket(SOCKET_NAME);
        System.out.println("pilotdroid-agent listening on localabstract:" + SOCKET_NAME);
        for (;;) {
            LocalSocket client = server.accept();
            try {
                handleClient(injector, client);
            } finally {
                client.close();
            }
        }
    }

    private static void handleClient(Injector injector, LocalSocket client) throws IOException {
        BufferedReader in = new BufferedReader(
                new InputStreamReader(client.getInputStream(), StandardCharsets.UTF_8));
        OutputStream out = client.getOutputStream();
        String line;
        while ((line = in.readLine()) != null) {
            String resp = dispatch(injector, line.trim());
            out.write((resp + "\n").getBytes(StandardCharsets.UTF_8));
            out.flush();
            if (line.trim().equals("QUIT")) {
                return;
            }
        }
    }

    private static String dispatch(Injector injector, String line) {
        String[] tok = line.split("\\s+");
        switch (tok[0]) {
            case "HELLO":
                // capability bitmap, v0 textual form — binary HELLO_ACK in M2
                return "OK HELLO agent=skeleton inject=" + (injector.available() ? 1 : 0)
                        + " dump=TODO query=TODO screenshot=TODO";
            case "TAP":
                try {
                    float x = Float.parseFloat(tok[1]);
                    float y = Float.parseFloat(tok[2]);
                    long t = System.nanoTime();
                    boolean ok = injector.tap(x, y);
                    return "OK inject=" + ok + " ms=" + (System.nanoTime() - t) / 1e6;
                } catch (RuntimeException e) {
                    return "ERR bad TAP args";
                }
            case "PING":
                return "PONG";
            case "DUMP":
                // TODO(spike-2): resident UiAutomation dump — gated on spike-2
                // process-model decision (docs/04, docs/05 M1).
                return "ERR unsupported: DUMP pending spike-2";
            case "QUERY":
                // TODO(spike-2): selector/xpath over resident UI tree.
                return "ERR unsupported: QUERY pending spike-2";
            case "QUIT":
                return "BYE";
            default:
                return "ERR unknown op: " + tok[0];
        }
    }

    /**
     * MotionEvent injection via hidden APIs — the proven path from the spike.
     * Fallback chain (docs/03 §3): InputManagerGlobal → InputManager.
     */
    static final class Injector {
        private Object inputManager;   // InputManagerGlobal or InputManager singleton
        private Method injectMethod;   // injectInputEvent(InputEvent, int)

        Injector() {
            try {
                Class<?> cls;
                try {
                    cls = Class.forName("android.hardware.input.InputManagerGlobal");
                } catch (ClassNotFoundException e) {
                    cls = InputManager.class; // pre-API-31 singleton lives here
                }
                Method getInstance = cls.getDeclaredMethod("getInstance");
                inputManager = getInstance.invoke(null);
                injectMethod = cls.getDeclaredMethod(
                        "injectInputEvent", InputEvent.class, int.class);
                injectMethod.setAccessible(true);
            } catch (ReflectiveOperationException e) {
                inputManager = null;
                injectMethod = null;
            }
        }

        boolean available() {
            return inputManager != null && injectMethod != null;
        }

        /** mode 2 = wait_for_finish; returns the dispatch result. */
        boolean inject(MotionEvent ev) {
            if (!available()) return false;
            try {
                return (Boolean) injectMethod.invoke(inputManager, ev, 2);
            } catch (ReflectiveOperationException | ClassCastException e) {
                return false;
            } finally {
                ev.recycle();
            }
        }

        boolean tap(float x, float y) {
            long now = SystemClock.uptimeMillis();
            MotionEvent down = MotionEvent.obtain(
                    now, now, MotionEvent.ACTION_DOWN, x, y, 0);
            MotionEvent up = MotionEvent.obtain(
                    now, now, MotionEvent.ACTION_UP, x, y, 0);
            boolean ok = inject(down) && inject(up);
            return ok;
        }
    }
}
