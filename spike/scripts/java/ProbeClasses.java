import java.lang.reflect.Method;

/** Runtime class/method lister: ProbeClasses <fqcn> [fqcn...] */
public final class ProbeClasses {
    public static void main(String[] args) {
        for (String name : args) {
            System.out.println("### " + name);
            try {
                Class<?> c = Class.forName(name);
                System.out.println("  loaded");
                for (Method m : c.getDeclaredMethods()) {
                    System.out.println("    " + java.lang.reflect.Modifier.toString(m.getModifiers())
                            + " " + m.getName() + "(" + join(m.getParameterTypes()) + ") -> "
                            + m.getReturnType().getSimpleName());
                }
            } catch (Throwable t) {
                System.out.println("  FAIL " + t);
            }
        }
    }

    private static String join(Class<?>[] ts) {
        StringBuilder b = new StringBuilder();
        for (Class<?> t : ts) {
            if (b.length() > 0) b.append(", ");
            b.append(t.getSimpleName());
        }
        return b.toString();
    }
}
