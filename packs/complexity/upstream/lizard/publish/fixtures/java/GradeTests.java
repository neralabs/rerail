package corpus;

/** A test class by its name: out of the rule's scope, whatever it scores. */
public class GradeTests {
    String grade(int score, boolean late, boolean extra) {
        if (score > 90 && !late) {
            return "A";
        }
        if (score > 80 || extra) {
            return "B";
        }
        if (score > 70) {
            return "C";
        }
        for (int i = 0; i < 3; i++) {
            if (i > score) {
                return "D";
            }
        }
        return "F";
    }
}
