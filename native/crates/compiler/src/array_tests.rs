//! Array programs' format (`array`): the text round trip, the verifier's refusals, and the
//! structural guard that every operation is a data-parallel primitive.

use crate::array::{ArrayOp, ArrayProgram, Combine, MapOp, Primitive};
use crate::rir_asm::{assemble, disassemble};

fn program(rows: &str) -> Result<crate::program::Words, String> {
    assemble(&format!("program -\n  mode set=0 query=0\n{rows}  0000 STOP 0\n"))
}

/// One of every operation, a function of each place, and a fixed point.
const EVERY: &str = "  array 0 shape 2 1
  array 0 fun f0 words if_value fields=1
  array 0 fun f1 host file fields=2
  array 0 %0 = input 0
  array 0 %1 = input 1
  array 0 %2 = param 0
  array 0 %3 = const -7
  array 0 %4 = iota %0
  array 0 %5 = len %0
  array 0 %6 = map shr %0 %3
  array 0 %7 = select %6 %0 %1
  array 0 %8 = scan add excl %0 seg %6
  array 0 %9 = scan max %8
  array 0 %10 = sort %0 %1
  array 0 %11 = gather %0 %10
  array 0 %12 = scatter add %0 %10 %5 %3
  array 0 %13 = search le %11 %1
  array 0 %14 = compact %6
  array 0 %15 = expand %1
  array 0 %16 = call f0 %0 %2
  array 0 %17 = part %16 0
  array 0 %18 = call f1 %1
  array 0 %19 = part %18 1
  array 0 %20 = carry %0
  array 0 %21 = map add %20 %17
  array 0 rounds %20 %21
  array 0 output %21
  array 0 output %19
";

#[test]
fn every_operation_reads_back_as_it_was_written() {
    let p = program(EVERY).unwrap_or_else(|e| panic!("{e}"));
    let a = &p.array_programs[0];
    assert_eq!(a.ops.len(), 22);
    assert_eq!(a.ops[8], ArrayOp::Scan { how: Combine::Add, excl: true, a: 0, seg: Some(6) });
    assert_eq!(a.ops[6], ArrayOp::Map(MapOp::Shr, 0, 3));
    assert_eq!(a.rounds, vec![vec![(20, 21)]]);
    // The dump prints the same rows, and they assemble to the same program.
    let text = disassemble(&p);
    assert!(text.contains(EVERY), "{text}");
    let again = assemble(&text).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(again.array_programs, p.array_programs);
    assert_eq!(a.canonical_bytes(), again.array_programs[0].canonical_bytes());
}

#[test]
fn the_verifier_refuses_what_no_executor_can_run() {
    let refused = |rows: &str| {
        let text = format!("  array 0 shape 1 1\n  array 0 fun f0 words w fields=1\n  array 0 %0 = input 0\n{rows}");
        assert!(program(&text).is_err(), "accepted:\n{text}");
    };
    // A later operand, an undeclared input, parameter or function, a field past the function's.
    refused("  array 0 %1 = map add %0 %1\n  array 0 output %1\n");
    refused("  array 0 %1 = input 1\n  array 0 output %1\n");
    refused("  array 0 %1 = param 1\n  array 0 output %1\n");
    refused("  array 0 %1 = call f1 %0\n  array 0 output %1\n");
    refused("  array 0 %1 = call f0 %0\n  array 0 %2 = part %1 1\n  array 0 output %2\n");
    // A `part` of a value that is no call; a scan that sets; a sort of no or too many keys.
    refused("  array 0 %1 = part %0 0\n  array 0 output %1\n");
    refused("  array 0 %1 = scan set %0\n  array 0 output %1\n");
    refused("  array 0 %1 = sort\n  array 0 output %1\n");
    refused("  array 0 %1 = sort %0 %0 %0 %0\n  array 0 output %1\n");
    // No output; a carry in no group; a rounds pair whose carry is not one.
    refused("  array 0 %1 = iota %0\n");
    refused("  array 0 %1 = carry %0\n  array 0 output %1\n");
    refused("  array 0 %1 = iota %0\n  array 0 rounds %1 %0\n  array 0 output %1\n");
    // Rows out of order.
    refused("  array 0 %2 = iota %0\n  array 0 output %0\n");
    // And the same rows, well formed, are accepted.
    let ok = "  array 0 shape 1 1\n  array 0 fun f0 words w fields=1\n  array 0 %0 = input 0\n  array 0 %1 = call f0 %0\n  array 0 %2 = part %1 0\n  array 0 output %2\n";
    program(ok).unwrap_or_else(|e| panic!("{e}"));
}

/// The structural guard: every operation is one of the device's primitives, and only `call`
/// runs work of its own an element. A new operation must name its primitive here.
#[test]
fn every_operation_is_a_data_parallel_primitive() {
    let p = program(EVERY).unwrap();
    let a: &ArrayProgram = &p.array_programs[0];
    for (i, op) in a.ops.iter().enumerate() {
        let prim = op.primitive();
        assert_eq!(prim == Primitive::Call, matches!(op, ArrayOp::Call(..)), "%{i} = {}", op.text());
    }
    // `EVERY` holds every operation, so a new one fails here until it is added there.
    let mut kinds: Vec<String> = a.ops.iter().map(|op| op.text().split(' ').next().unwrap().to_owned()).collect();
    kinds.sort();
    kinds.dedup();
    assert_eq!(kinds.len(), 17, "{kinds:?}");
}

/// Program 0 of `rows` (shape `inputs params`), run on `inputs`: its outputs.
fn run(shape: &str, rows: &[&str], inputs: &[&[i64]], params: &[i64], calls: &mut dyn crate::array::Calls) -> Result<Vec<Vec<i64>>, &'static str> {
    let mut text = format!("  array 0 shape {shape}\n");
    for r in rows {
        text.push_str(&format!("  array 0 {r}\n"));
    }
    let p = program(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    p.array_programs[0].run(inputs, params, calls)
}

/// Each operation's meaning on a small case.
#[test]
fn each_operation_means_what_its_row_says() {
    let one = |op: &str, inputs: &[&[i64]]| -> Result<Vec<i64>, &'static str> {
        let mut rows: Vec<String> = (0..inputs.len()).map(|k| format!("%{k} = input {k}")).collect();
        rows.push(format!("%{} = const 10", inputs.len()));
        rows.push(format!("%{} = {op}", inputs.len() + 1));
        rows.push(format!("output %{}", inputs.len() + 1));
        let rows: Vec<&str> = rows.iter().map(String::as_str).collect();
        run(&format!("{} 0", inputs.len()), &rows, inputs, &[], &mut ()).map(|mut o| o.remove(0))
    };
    let x: &[i64] = &[3, 1, 4, 1, 5];
    let f: &[i64] = &[1, 0, 0, 1, 0];
    assert_eq!(one("iota %0", &[x]), Ok(vec![0, 1, 2, 3, 4]));
    assert_eq!(one("len %0", &[x]), Ok(vec![5]));
    assert_eq!(one("map add %0 %1", &[x]), Ok(vec![13, 11, 14, 11, 15]), "a scalar broadcasts");
    assert_eq!(one("map lt %0 %1", &[x, &[3, 3, 3, 3, 3]]), Ok(vec![0, 1, 0, 1, 0]));
    assert_eq!(one("map shl %0 %1", &[x, &[1]]), Ok(vec![6, 2, 8, 2, 10]));
    assert_eq!(one("select %1 %0 %2", &[x, f]), Ok(vec![3, 10, 10, 1, 10]));
    assert_eq!(one("scan add %0", &[x]), Ok(vec![3, 4, 8, 9, 14]));
    assert_eq!(one("scan add excl %0", &[x]), Ok(vec![0, 3, 4, 8, 9]));
    assert_eq!(one("scan max %0 seg %1", &[x, f]), Ok(vec![3, 3, 4, 1, 5]), "a flag starts a new scan");
    assert_eq!(one("scan add excl %0 seg %1", &[x, f]), Ok(vec![0, 3, 4, 0, 1]));
    assert_eq!(one("sort %0", &[x]), Ok(vec![1, 3, 0, 2, 4]), "stable: the first 1 first");
    assert_eq!(one("sort %1 %0", &[x, f]), Ok(vec![1, 2, 4, 3, 0]), "the first key most significant");
    assert_eq!(one("gather %0 %1", &[x, &[4, 0, 0]]), Ok(vec![5, 3, 3]));
    assert_eq!(one("gather %0 %1", &[x, &[5]]), Err("a gather's index outside its array"));
    assert_eq!(one("scatter add %0 %1 %2 %3", &[x, &[0, 1, 0, 9, 1], &[3]]), Ok(vec![17, 16, 10]), "an index outside writes nothing");
    assert_eq!(one("scatter set %0 %1 %2 %2", &[x, &[4, 3, 2, 1, 0], &[5]]), Ok(vec![5, 1, 4, 1, 3]));
    assert_eq!(one("scatter set %0 %1 %2 %2", &[x, &[0, 0, 1, 2, 3], &[5]]), Err("two writes of a set scatter to one element"));
    assert_eq!(one("search lt %0 %1", &[&[1, 3, 3, 7], &[0, 3, 4, 9]]), Ok(vec![0, 1, 3, 4]));
    assert_eq!(one("search le %0 %1", &[&[1, 3, 3, 7], &[0, 3, 4, 9]]), Ok(vec![0, 3, 3, 4]));
    assert_eq!(one("search le %0 %1", &[x, &[1]]), Err("a search of an array not sorted"));
    assert_eq!(one("compact %0", &[f]), Ok(vec![0, 3]));
    assert_eq!(one("expand %0", &[&[2, 0, 1]]), Ok(vec![0, 0, 2]));
    assert_eq!(one("map add %0 %1", &[x, &[1, 2]]), Err("operands of unequal lengths"));
}

/// Calls: a function an element, its results ragged, read field by field.
#[test]
fn a_call_runs_its_function_on_each_element_alone() {
    struct Repeat;
    impl crate::array::Calls for Repeat {
        // Element `x`: `x` results, each (x, k).
        fn call(&mut self, f: u16, fun: &crate::array::Fun, args: &[i64], out: &mut Vec<i64>) -> Result<(), &'static str> {
            assert_eq!((f, fun.name.as_str(), fun.fields), (0, "repeat", 2));
            for k in 0..args[0] {
                out.extend([args[0], k + args[1]]);
            }
            Ok(())
        }
    }
    let rows = ["fun f0 words repeat fields=2", "%0 = input 0", "%1 = param 0", "%2 = call f0 %0 %1", "%3 = part %2 0", "%4 = part %2 1", "output %2", "output %3", "output %4"];
    let out = run("1 1", &rows, &[&[2, 0, 1]], &[100], &mut Repeat).unwrap();
    assert_eq!(out, vec![vec![2, 0, 1], vec![2, 2, 1], vec![100, 101, 100]]);
}

/// A fixed point: pointer jumping to each element's root, the shape of a group's activity.
#[test]
fn rounds_run_until_every_carry_is_its_next() {
    // parent: 0 is the root; 1 and 2 under 0; 3 under 2; 4 under 3.
    let rows = ["%0 = input 0", "%1 = carry %0", "%2 = gather %1 %1", "rounds %1 %2", "output %1"];
    let text: String = rows.iter().map(|r| format!("  array 0 {r}\n")).collect();
    let p = program(&format!("  array 0 shape 1 0\n{text}")).unwrap();
    let v = p.array_programs[0].values(&[&[0, 0, 0, 2, 3]], &[], &mut ()).unwrap();
    assert_eq!(v.arrays[1], vec![0, 0, 0, 0, 0]);
    // Jumps of 1 and 2 reach the roots; the third round's next is its carry.
    assert_eq!(v.rounds, 3);
    // A carry that never settles is refused at the bound.
    let flip = ["%0 = input 0", "%1 = const 1", "%2 = carry %0", "%3 = map xor %2 %1", "rounds %2 %3", "output %2"];
    assert_eq!(run("1 0", &flip, &[&[0]], &[], &mut ()), Err("past the rounds' bound"));
}
