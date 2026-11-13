package corpus;

import java.util.List;

/** Java 14 to 21: switch expressions, arrows, patterns, records, sealed types, var. */
public class Modern {
    sealed interface Shape permits Circle, Square, Rect {}

    record Circle(double radius) implements Shape {}

    record Square(double side) implements Shape {}

    record Rect(double width, double height) implements Shape {
        Rect {
            if (width < 0 || height < 0) {
                throw new IllegalArgumentException("negative");
            }
        }

        double area() {
            return width > 0 && height > 0 ? width * height : 0;
        }
    }

    String arrows(int day) {
        return switch (day) {
            case 1, 7 -> "weekend";
            case 2 -> "Mon";
            case 3 -> {
                String name = "Tue";
                yield name;
            }
            default -> "midweek";
        };
    }

    int arrowStatement(String kind) {
        int total = 0;
        switch (kind) {
            case "a" -> total += 1;
            case "b" -> total += 2;
            default -> total += 0;
        }
        return total;
    }

    double patterns(Shape shape) {
        return switch (shape) {
            case Circle c when c.radius() > 10 -> 314.0;
            case Circle c -> Math.PI * c.radius() * c.radius();
            case Square s -> s.side() * s.side();
            case Rect r -> r.area();
        };
    }

    String instanceOf(Object value) {
        if (value instanceof String s && !s.isEmpty()) {
            return s;
        }
        if (value instanceof Integer i) {
            return i > 0 ? "positive" : "other";
        }
        return "";
    }

    int inferred(List<String> names) {
        var total = 0;
        for (var name : names) {
            var length = name.length();
            if (length > 3) {
                total += length;
            }
        }
        return total;
    }

    String defaultOnlyArrow(int day) {
        return switch (day) {
            default -> "any";
        };
    }

    int overLimit(Shape shape, List<String> tags, boolean strict) {
        int score = 0;
        if (shape instanceof Circle c && c.radius() > 1) {
            score += 1;
        }
        for (String tag : tags) {
            if (tag.isEmpty() || tag.isBlank()) {
                continue;
            }
            score += switch (tag) {
                case "x" -> 1;
                case "y" -> 2;
                default -> strict ? 0 : 3;
            };
        }
        return score;
    }
}
