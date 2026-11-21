fn main() {
    let mut args = std::env::args().skip(1);
    let directory = args
        .next()
        .expect("usage: rerail-sql-preparation-lab <capture-directory> <trial: 0..2>");
    let trial = args
        .next()
        .expect("trial: 0..2")
        .parse::<usize>()
        .expect("integer trial");
    assert!(
        trial < 3 && args.next().is_none(),
        "usage: rerail-sql-preparation-lab <capture-directory> <trial: 0..2>"
    );
    rerail_sql_preparation_lab::run(std::path::Path::new(&directory), trial);
}
