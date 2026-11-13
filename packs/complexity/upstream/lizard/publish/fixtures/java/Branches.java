package corpus;

import java.io.IOException;
import java.io.Reader;
import java.util.Iterator;
import java.util.List;

/** One function per decision point the rule counts, then one that mixes them. */
public class Branches {
    int ifElse(int x) {
        if (x > 0) {
            return 1;
        } else if (x < 0) {
            return -1;
        } else {
            return 0;
        }
    }

    int loops(List<Integer> xs, int n) {
        int total = 0;
        for (int i = 0; i < n; i++) {
            total += i;
        }
        for (int x : xs) {
            total += x;
        }
        while (total > 100) {
            total /= 2;
        }
        do {
            total++;
        } while (total < 10);
        return total;
    }

    int catches(Reader reader) {
        try {
            return reader.read();
        } catch (IOException e) {
            return -1;
        } catch (IllegalStateException | IllegalArgumentException e) {
            return -2;
        } finally {
            System.out.println("done");
        }
    }

    int tryWithResources(Reader source) throws IOException {
        try (Reader reader = source) {
            return reader.read();
        }
    }

    int logical(boolean a, boolean b, boolean c, boolean d) {
        if (a && b || c && !d) {
            return 1;
        }
        return a & b | c ^ d ? 2 : 3;
    }

    int ternaries(int x) {
        int sign = x > 0 ? 1 : x < 0 ? -1 : 0;
        return sign == 0 ? 0 : sign * 2;
    }

    String switchCountsOnce(int day) {
        switch (day) {
            case 1:
                return "Mon";
            case 2:
            case 3:
                return "Tue or Wed";
            case 4: {
                return "Thu";
            }
            default:
                return "other";
        }
    }

    String switchDefaultOnly(int day) {
        switch (day) {
            default:
                return "any";
        }
    }

    int twoSwitches(int a, int b) {
        int total = 0;
        switch (a) {
            case 1:
                total += 1;
                break;
            case 2:
                total += 2;
                break;
        }
        switch (b) {
            case 1:
                total += 10;
                break;
            default:
                break;
        }
        return total;
    }

    int defaultOnlyAroundCasedSwitch(int a, int b) {
        switch (a) {
            default:
                switch (b) {
                    case 1:
                        return 1;
                    case 2:
                        return 2;
                    default:
                        return 0;
                }
        }
    }

    int nestedSwitches(int a, int b) {
        switch (a) {
            case 1:
                switch (b) {
                    case 1:
                        return 11;
                    case 2:
                        return 12;
                    default:
                        return 10;
                }
            case 2:
                return 2;
            default:
                return 0;
        }
    }

    String switchOverLimit(int day, boolean holiday, boolean late, List<String> notes) {
        if (holiday && !late) {
            return "off";
        }
        if (notes == null || notes.isEmpty()) {
            return "plain";
        }
        for (String note : notes) {
            if (note.isBlank()) {
                return "blank";
            }
        }
        switch (day) {
            case 1:
                return "Mon";
            case 2:
                return "Tue";
            case 3:
                return "Wed";
            default:
                return "other";
        }
    }

    int labelled(int[][] grid) {
        int found = 0;
        outer:
        for (int[] row : grid) {
            for (int cell : row) {
                if (cell < 0) {
                    continue outer;
                }
                if (cell == 0) {
                    break outer;
                }
                found++;
            }
        }
        return found;
    }

    int notDecisions(Object lock, int x) {
        assert x >= 0 : "negative";
        synchronized (lock) {
            x++;
        }
        boolean big = x > 10;
        return big == true ? x : -x;
    }

    int mixed(Iterator<String> names, int limit, boolean strict) {
        int count = 0;
        while (names.hasNext() && count < limit) {
            String name = names.next();
            if (name == null || name.isEmpty()) {
                if (strict) {
                    throw new IllegalArgumentException("empty");
                }
                continue;
            }
            switch (name.charAt(0)) {
                case 'a':
                case 'b':
                    count += 2;
                    break;
                default:
                    count += name.length() > 3 ? 1 : 0;
            }
        }
        return count;
    }
}
