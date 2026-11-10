use super::*;

fn compiled(pattern: &str) -> Arc<Regex> {
    compile(pattern).expect("the pattern compiles")
}

#[test]
fn a_pattern_compiles_once_under_a_sharing_and_each_time_outside_one() {
    assert!(!Arc::ptr_eq(&compiled("^a$"), &compiled("^a$")));
    let sharing = Sharing::begin();
    let shared = compiled("^a$");
    assert!(Arc::ptr_eq(&shared, &compiled("^a$")));
    assert!(!Arc::ptr_eq(&shared, &compiled("^b$")));
    assert!(compile("(").is_none() && compile("(").is_none());
    // An inner `Sharing` joins the outer one; only the outermost's end lets the regexes go.
    drop(Sharing::begin());
    assert!(Arc::ptr_eq(&shared, &compiled("^a$")));
    drop(sharing);
    assert!(!Arc::ptr_eq(&shared, &compiled("^a$")));
}
