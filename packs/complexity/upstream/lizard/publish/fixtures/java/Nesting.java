package corpus;

import java.util.Comparator;
import java.util.List;
import java.util.function.Function;
import java.util.function.Predicate;

/** Functions inside functions: lambdas, anonymous classes, local and nested classes. */
public class Nesting {
    private final Predicate<String> field = s -> s != null && !s.isEmpty() && s.length() < 10;

    static {
        if (System.getenv("CORPUS") != null && System.getenv("CORPUS").isEmpty()) {
            System.out.println("static initializer");
        }
    }

    {
        if (field == null) {
            throw new IllegalStateException();
        }
    }

    int lambdas(List<String> names) {
        Function<String, Integer> score = name -> {
            if (name == null) {
                return 0;
            }
            if (name.isEmpty() || name.isBlank()) {
                return 1;
            }
            return name.length() > 5 ? 2 : 3;
        };
        int total = 0;
        for (String name : names) {
            total += score.apply(name);
        }
        return total;
    }

    Comparator<String> anonymous(boolean reverse) {
        if (reverse) {
            return Comparator.reverseOrder();
        }
        return new Comparator<String>() {
            @Override
            public int compare(String a, String b) {
                if (a == null && b == null) {
                    return 0;
                }
                if (a == null) {
                    return -1;
                }
                if (b == null) {
                    return 1;
                }
                if (a.length() != b.length()) {
                    return a.length() < b.length() ? -1 : 1;
                }
                return a.compareTo(b);
            }
        };
    }

    int afterAnonymous(List<String> names) {
        Runnable noop = new Runnable() {
            public void run() {
                if (names.isEmpty()) {
                    System.out.println("empty");
                }
            }
        };
        noop.run();
        int total = 0;
        for (String name : names) {
            if (name.length() > 3) {
                total++;
            }
        }
        return total;
    }

    int localClass(int seed) {
        class Counter {
            int count(int n) {
                int c = 0;
                for (int i = 0; i < n; i++) {
                    if (i % 2 == 0 || i % 3 == 0) {
                        c++;
                    }
                }
                return c;
            }
        }
        return seed > 0 ? new Counter().count(seed) : 0;
    }

    static class Inner {
        int depth(int n) {
            if (n > 3) {
                if (n > 6) {
                    if (n > 9) {
                        return 3;
                    }
                    return 2;
                }
                return 1;
            }
            return 0;
        }

        class Deeper {
            boolean check(int a, int b) {
                return a > 0 && b > 0;
            }
        }
    }

    enum Level {
        LOW {
            int weight(int x) {
                return x > 0 ? 1 : 0;
            }
        },
        HIGH {
            int weight(int x) {
                if (x > 10) {
                    return 3;
                }
                return x > 0 ? 2 : 0;
            }
        };

        abstract int weight(int x);
    }
}
