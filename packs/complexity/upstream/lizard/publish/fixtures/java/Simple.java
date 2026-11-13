package corpus;

import java.util.List;

/** Functions around the ceiling: 1, 6 (at the limit) and 7 (over it). */
public class Simple {
    private final int base;

    public Simple(int base) {
        this.base = base;
    }

    public int identity(int value) {
        return value + base;
    }

    public String atLimit(int score, boolean late, boolean extra) {
        if (score > 90) {
            return "A";
        }
        if (score > 80) {
            return "B";
        }
        if (late) {
            return "L";
        }
        if (extra) {
            return "E";
        }
        if (score > 70) {
            return "C";
        }
        return "F";
    }

    public String overLimit(int score, boolean late, boolean extra, List<Integer> curve) {
        if (score > 90 && !late) {
            return "A";
        }
        if (score > 80 || extra) {
            return "B";
        }
        if (score > 70) {
            return "C";
        }
        for (int bonus : curve) {
            score += bonus;
        }
        return "F";
    }

    public static int sum(int[] values) {
        int total = 0;
        for (int value : values) {
            total += value;
        }
        return total;
    }

    abstract static class Shape {
        abstract double area();

        double scaled(double factor) {
            return factor > 0 ? area() * factor : 0;
        }
    }

    interface Grader {
        String grade(int score);

        default String describe(int score) {
            if (score < 0) {
                return "invalid";
            }
            return grade(score);
        }
    }
}
