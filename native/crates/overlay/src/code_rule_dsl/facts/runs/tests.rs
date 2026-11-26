use super::*;

#[test]
fn small_runs_retain_space_for_live_rows_only() {
    let mut builder = Builder::new(7, 32, 1);
    for row in 0..6u32 {
        builder.push(&(0..32).map(|column| row * 32 + column).collect::<Vec<_>>());
    }
    let run = builder.seal();
    assert_eq!((run.len(), run.cell(5, 31)), (6, 191));
    assert!(run.bytes() < 8192, "six rows retain {} bytes", run.bytes());
}

#[test]
fn sealing_sorts_by_key_and_keeps_the_last_row_per_key() {
    let mut builder = Builder::new(1, 3, 2);
    for row in [[2, 1, 10], [1, 5, 11], [2, 1, 12], [1, 2, 13], [1, 5, 14]] {
        builder.push(&row);
    }
    let run = builder.seal();
    let rows = (0..run.len()).map(|r| run.row(r).collect::<Vec<_>>()).collect::<Vec<_>>();
    assert_eq!(rows, [vec![1, 2, 13], vec![1, 5, 14], vec![2, 1, 12]]);
    assert_eq!(run.find(&[1, 5]), Some(1));
    assert_eq!(run.find(&[1, 3]), None);
    assert_eq!(run.range(&[1]), 0..2);
    assert_eq!(run.range(&[3]), 3..3);
}

#[test]
fn runs_round_trip_through_their_flat_encoding() {
    let mut builder = Builder::new(9, 9, 1);
    let digest = [0xab; 32];
    for name in [4, 1, 3] {
        let mut row = vec![name];
        row.extend(lanes(&digest));
        builder.push(&row);
    }
    let run = builder.seal();
    let mut bytes = Vec::new();
    run.encode(&mut bytes).unwrap();
    assert_eq!(bytes.len(), 8 + 16 + 9 * 3 * 4);
    let decoded = Run::decode(&mut bytes.as_slice()).unwrap();
    assert_eq!(decoded, run);
    assert_eq!(super::digest(decoded.row(0).skip(1)), digest);
    bytes[0] = b'x';
    assert!(Run::decode(&mut bytes.as_slice()).is_err());
}
