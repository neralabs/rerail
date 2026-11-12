/** A Maven test source: out of the rule's scope, whatever it scores. */
public class ExcludedTest {
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
