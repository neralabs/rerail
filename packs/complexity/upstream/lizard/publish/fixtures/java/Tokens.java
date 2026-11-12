package corpus;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.function.Supplier;

/** Text that looks like a decision point and is not one: generics, strings, characters, comments, annotations. */
public class Tokens {
    // if (a && b || c) { for (;;) { while (true) { } } } case 1: x ? y : z
    /* catch (Exception e) { if (x) { } } */

    Map<String, ?> wildcards(List<? extends Number> numbers, List<? super Integer> sink) {
        List<?> any = new ArrayList<>(numbers);
        return Map.of("size", any.size());
    }

    int nestedWildcard(List<Integer> numbers) {
        List<? extends Comparable<?>> view = numbers;
        return view.size();
    }

    <T extends Comparable<? super T>> T generic(T a, T b) {
        return a.compareTo(b) >= 0 ? a : b;
    }

    String strings() {
        String a = "if (x && y || z) ? 1 : 0";
        String b = "for while catch case";
        char question = '?';
        char amp = '&';
        String escaped = "quote \" if (a || b) \" end";
        return a + b + question + amp + escaped;
    }

    String textBlock() {
        return """
            if (a && b) {
                while (c || d) { }
            }
            case "x" ? 1 : 2
            """;
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    @Deprecated(since = "1", forRemoval = false)
    int annotated(@SuppressWarnings("unused") int value) {
        return value;
    }

    Supplier<List<String>> references() {
        Supplier<List<String>> make = ArrayList::new;
        return make;
    }

    int bitwise(int a, int b) {
        int x = a & b;
        int y = a | b;
        x &= y;
        y |= x;
        return x ^ y;
    }

    boolean identifiers(boolean ifTrue, boolean forAll, boolean caseOne) {
        boolean whileTrue = ifTrue;
        return whileTrue == forAll == caseOne;
    }

    int realDecisions(String s) {
        if (s == null || s.equals("if (x) { }")) {
            return 0;
        }
        return s.contains("?") ? 1 : 2;
    }
}
