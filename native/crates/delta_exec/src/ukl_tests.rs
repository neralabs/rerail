//! The UKL versions' checks that need no device: the layout header the kernels read is the twins'.

/// The checked-in layout header is the one the twins' constants give.
#[test]
fn the_layout_header_is_the_twins() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../ukl/lib/ukl/delta/layout.h");
    let want = super::layout_h();
    if std::env::var_os("RERAIL_WRITE_LAYOUT").is_some() {
        std::fs::write(path, &want).unwrap();
    }
    assert_eq!(include_str!("../../ukl/lib/ukl/delta/layout.h"), want, "RERAIL_WRITE_LAYOUT=1 writes it again");
}
