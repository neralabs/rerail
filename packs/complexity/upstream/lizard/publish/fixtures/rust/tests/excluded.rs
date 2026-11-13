// An integration test (tests/): excluded from the rule even though lizard scores it over the limit.

fn tests_folder_helper(code: u32) -> &'static str {
    if code == 1 {
        return "one";
    }
    if code == 2 {
        return "two";
    }
    if code == 3 {
        return "three";
    }
    if code == 4 {
        return "four";
    }
    if code == 5 {
        return "five";
    }
    if code == 6 {
        return "six";
    }
    if code == 7 {
        return "seven";
    }
    "many"
}
