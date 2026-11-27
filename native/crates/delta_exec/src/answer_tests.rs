//! Δanswer's operations: `answer_load` and `parse`, each run as an operation over its slots.

use super::*;
use crate::files::tests::{root, write};

fn ops() -> (crate::op::Runner, Directory) {
    let reg = crate::op::Runner::new().unwrap();
    (reg, Directory::default())
}

/// `answer_load` answers each path of its plan as the file layer does: a changed file read once and placed in a room
/// of the slab, an unchanged one no δ (not read), a missing one a miss; a file past the slab's room asks for more.
#[test]
fn answer_load_answers_each_path_of_its_plan() {
    let _racy = crate::files::tests::racy_window();
    let (root, store) = root("answer-load");
    write(&root.join("a.rerail"), "e(1, 2).\n", 30);
    let (reg, mut dir) = ops();
    let observer = store.observer();
    // A run's bounds, as `World::begin` and `end` give them: the observer's answers kept at the end.
    let run = |f: &mut dyn FnMut()| {
        observe::with(Some(&observer), |o| {
            o.begin();
            o.watch(std::slice::from_ref(&root));
        });
        f();
        observe::with(Some(&observer), |o| o.flush());
    };
    let (slab, _) = dir.slab_room(kind::TAPE, room(0)).unwrap();
    let ask = |d, path: &str, before| Ask { d, path: path.into(), before, flags: 0 };
    let asks = [ask(1, "a.rerail", None), ask(2, "bootstrap-l0.rerail", None), ask(3, "missing.rerail", None)];
    let mut first = None;
    run(&mut || first = Some(answer_load(&reg, &mut dir, slab, &root, &observer, &asks).unwrap()));
    let (facts, reads) = first.unwrap();
    assert_eq!(reads, 2);
    assert_eq!(facts.iter().map(|f| (f[0], f[1])).collect::<Vec<_>>(), vec![(1, code::READ), (2, code::READ), (3, code::MISS)]);
    dir.seal(slab).unwrap();
    {
        let l = dir.read("t", &[slab]).unwrap();
        let f = &facts[0];
        assert_eq!(&l.block(0)[f[7] as usize..(f[7] + f[8]) as usize], b"e(1, 2).\n", "placed where the fact says");
        assert_eq!(rerail_blocks::blob::BlobId::of_words(&f[3..7]), rerail_blocks::blob::BlobId::of(b"e(1, 2).\n"));
    }
    // Asked again past the racy window, with the digests they were answered with: no δ, nothing read.
    crate::files::tests::past_racy_window();
    let g = |f: &[i64; FACT_WORDS]| rerail_blocks::blob::BlobId::of_words(&f[3..7]).0;
    let again = [ask(1, "a.rerail", Some(g(&facts[0]))), ask(2, "bootstrap-l0.rerail", Some(g(&facts[1])))];
    let (slab2, _) = dir.slab_room(kind::TAPE, room(0)).unwrap();
    let mut second = None;
    run(&mut || second = Some(answer_load(&reg, &mut dir, slab2, &root, &observer, &again).unwrap()));
    let (facts2, reads2) = second.unwrap();
    assert_eq!((reads2, facts2[0][1], facts2[1][1]), (0, code::SAME, code::SAME));
    // A slab with less room than a file needs: the fact asks for it, and nothing is placed.
    write(&root.join("big.rerail"), &"e(1, 2).\n".repeat(2000), 30);
    let mut small = Directory::default().with_slab_bytes(1);
    let (tiny, _) = small.slab_room(kind::TAPE, 8).unwrap();
    let (facts3, _) = answer_load(&reg, &mut small, tiny, &root, &observer, &[ask(4, "big.rerail", None)]).unwrap();
    assert_eq!((facts3[0][1], facts3[0][9] as usize), (code::NO_ROOM, room(18000)));
}

/// `parse` parses each source where it lies, its tape into a room of the same slab: the same tapes at every tiling
/// and order (its documents are independent), and a source that is not UTF-8 refused.
#[test]
fn parse_gives_the_same_tapes_at_every_tiling() {
    let srcs: [&[u8]; 3] = [b"e(1, 2).\n", b"p(x) :- e(x, _).\n", b"\xff\xfe"];
    let run = |tiling: Tiling| {
        let (reg, mut dir) = ops();
        let (slab, b) = dir.slab_room(kind::TAPE, room(64)).unwrap();
        let places: Vec<(usize, usize)> = srcs
            .iter()
            .map(|s| {
                let h = crate::files::place_in(&b, s.len(), &mut |out| {
                    out.copy_from_slice(s);
                    Ok(())
                })
                .unwrap();
                crate::files::room_at(&h).unwrap()
            })
            .collect();
        drop(b);
        let control = parse_control(&mut dir, &places).unwrap();
        reg.run(&dir, &PARSE, &[slab, control], &[], tiling).unwrap();
        let l = dir.read("t", &[slab, control]).unwrap();
        (0..srcs.len())
            .map(|k| {
                let c = l.block(1);
                let (at, len, code) = (word(c, 3 + PARSE_WORDS * k) as usize, word(c, 4 + PARSE_WORDS * k) as usize, word(c, 5 + PARSE_WORDS * k));
                (code, l.block(0)[at..at + len].to_vec())
            })
            .collect::<Vec<_>>()
    };
    let whole = run(Tiling::WHOLE);
    assert_eq!(whole[2].0, NOT_UTF8);
    assert!(whole[0].0 == 0 && whole[1].0 == 0 && !whole[0].1.is_empty() && whole[0].1 != whole[1].1);
    for tiling in [Tiling { tile: 1, seed: 0 }, Tiling { tile: 1, seed: 7 }, Tiling { tile: 2, seed: 3 }] {
        assert_eq!(run(tiling), whole, "{tiling:?}");
    }
}
