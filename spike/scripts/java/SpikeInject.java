import android.hardware.input.InputManager;
import android.os.SystemClock;
import android.view.InputDevice;
import android.view.InputEvent;
import android.view.MotionEvent;

import java.lang.reflect.Method;

/**
 * Spike: verify app_process + hidden InputManager injection path on a
 * SELinux-Enforcing device (the scrcpy-style fast path).
 *
 * Run:
 *   CLASSPATH=/data/local/tmp/spike.jar app_process /data/local/tmp SpikeInject <x> <y>
 */
public final class SpikeInject {
    private static final int MODE_ASYNC = 0;
    private static final int MODE_WAIT_FOR_FINISH = 2;
    private static final int INJECT_N = 30;

    public static void main(String[] args) throws Exception {
        long t0 = System.nanoTime();
        float x = args.length > 0 ? Float.parseFloat(args[0]) : 700f;
        float y = args.length > 1 ? Float.parseFloat(args[1]) : 1600f;

        Object im = getInputManager();
        Class<?> cls = im.getClass();
        Method inject = cls.getDeclaredMethod("injectInputEvent", InputEvent.class, int.class);
        inject.setAccessible(true);
        long t1 = System.nanoTime();
        System.out.println("init_ms=" + (t1 - t0) / 1e6);
        System.out.println("im_class=" + cls.getName());

        // warmup
        injectTap(im, inject, x, y, MODE_WAIT_FOR_FINISH);

        // ASYNC mode: measure pure enqueue cost
        long[] async = new long[INJECT_N];
        for (int i = 0; i < INJECT_N; i++) {
            long s = System.nanoTime();
            injectTap(im, inject, x, y, MODE_ASYNC);
            async[i] = System.nanoTime() - s;
        }

        // WAIT_FOR_FINISH: measure full dispatch latency per tap
        long[] sync = new long[INJECT_N];
        int ok = 0;
        for (int i = 0; i < INJECT_N; i++) {
            long s = System.nanoTime();
            Boolean r = (Boolean) injectTap(im, inject, x, y, MODE_WAIT_FOR_FINISH);
            sync[i] = System.nanoTime() - s;
            if (Boolean.TRUE.equals(r)) ok++;
        }
        System.out.println("async_per_tap_ms=" + stats(async));
        System.out.println("waitfinish_per_tap_ms=" + stats(sync));
        System.out.println("inject_ok=" + ok + "/" + INJECT_N);
        System.out.println("total_ms=" + (System.nanoTime() - t0) / 1e6);
    }

    private static Object getInputManager() throws Exception {
        // Android 12+ prefers InputManagerGlobal; fall back to InputManager.
        for (String name : new String[]{
                "android.hardware.input.InputManagerGlobal",
                "android.hardware.input.InputManager"}) {
            try {
                Class<?> c = Class.forName(name);
                Method m = c.getDeclaredMethod("getInstance");
                m.setAccessible(true);
                return m.invoke(null);
            } catch (Throwable ignore) {
            }
        }
        // Last resort: context-free InputManager via reflection ctor is not
        // possible; give up loudly.
        throw new IllegalStateException("no InputManager entry point");
    }

    private static Object injectTap(Object im, Method inject, float x, float y, int mode)
            throws Exception {
        long now = SystemClock.uptimeMillis();
        Object result = null;
        for (int action : new int[]{MotionEvent.ACTION_DOWN, MotionEvent.ACTION_UP}) {
            MotionEvent ev = MotionEvent.obtain(now, now, action, x, y, 0);
            ev.setSource(InputDevice.SOURCE_TOUCHSCREEN);
            result = inject.invoke(im, ev, mode);
            ev.recycle();
        }
        return result;
    }

    private static String stats(long[] ns) {
        java.util.Arrays.sort(ns);
        double mean = 0;
        for (long v : ns) mean += v;
        mean /= ns.length;
        return String.format(java.util.Locale.US,
                "mean=%.2f p50=%.2f p95=%.2f min=%.2f max=%.2f",
                mean / 1e6,
                ns[ns.length / 2] / 1e6,
                ns[(int) (ns.length * 0.95)] / 1e6,
                ns[0] / 1e6,
                ns[ns.length - 1] / 1e6);
    }
}
