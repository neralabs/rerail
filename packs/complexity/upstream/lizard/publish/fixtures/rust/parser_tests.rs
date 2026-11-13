// A unit-test module in a file of its own (*_tests.rs): excluded from the rule even though lizard scores it over the
// limit.

#[test]
fn parses_every_grade() {
    for score in [95, 85, 75, 65, 55] {
        if score > 90 {
            assert_eq!(grade(score), "A");
        } else if score > 80 {
            assert_eq!(grade(score), "B");
        } else if score > 70 {
            assert_eq!(grade(score), "C");
        } else if score > 60 {
            assert_eq!(grade(score), "D");
        } else if score > 50 || score == 0 {
            assert_eq!(grade(score), "F");
        }
    }
}
