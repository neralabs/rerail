//! Test-only index traversal alternatives. Production lookup remains unchanged.
use super::*;
#[derive(Clone, Copy, Debug)]
pub(crate) enum Mode { Point, Merge, Gallop, Adaptive }
#[derive(Default, serde::Serialize)]
pub(crate) struct Work { pub requests: usize, pub reads: usize, pub output: usize, pub merge: usize, pub gallop: usize, pub point: usize }
thread_local! { static WORK: std::cell::RefCell<Work> = std::cell::RefCell::new(Work::default()); }
pub(crate) fn take_work() -> Work { WORK.with(|w| std::mem::take(&mut *w.borrow_mut())) }
fn mode() -> Option<Mode> {
    static MODE: OnceLock<Option<Mode>> = OnceLock::new();
    *MODE.get_or_init(|| std::env::var("RERAIL_LOOKUP_SPIKE").ok().map(|s| match s.as_str() {
        "point" => Mode::Point, "merge" => Mode::Merge, "gallop" => Mode::Gallop, "adaptive" => Mode::Adaptive, _ => panic!("invalid lookup mode")
    }))
}
pub(super) fn selected(r: &Relation, column: usize, keys: &[i64]) -> Option<Vec<usize>> {
    let mode = mode()?;
    if keys.len() < 2 { return None; }
    static TRACE: OnceLock<bool> = OnceLock::new();
    let trace = *TRACE.get_or_init(|| std::env::var_os("RERAIL_LOOKUP_TRACE").is_some());
    let (rows, w) = if trace { run::<true>(r,column,keys,mode) } else { run::<false>(r,column,keys,mode) };
    if trace { WORK.with(|total| { let mut t=total.borrow_mut(); t.requests+=w.requests; t.reads+=w.reads; t.output+=w.output; t.point+=w.point; t.merge+=w.merge; t.gallop+=w.gallop; }); }
    Some(rows)
}
fn run<const TRACE: bool>(r: &Relation, column: usize, keys: &[i64], mut mode: Mode) -> (Vec<usize>,Work) {
    let index=&r.index(column).rows;
    let mut reads=0;
    let mut value=|i:usize| { if TRACE { reads+=1; } r.value(index[i] as usize,column) };
    let mut out=Vec::new();
    let mut work=Work { requests:keys.len(), ..Work::default() };
    if keys.is_empty() { return (out,work); }
    if keys.len()==1 {mode=Mode::Point;}
    if !matches!(mode,Mode::Point) && keys.windows(2).any(|w|w[0]>=w[1]) {mode=Mode::Point;}
    let bound=|mut lo:usize,mut hi:usize,key:i64,upper:bool,value:&mut dyn FnMut(usize)->i64| {
        while lo<hi {let mid=lo+(hi-lo)/2;let v=value(mid);if v<key || upper&&v==key {lo=mid+1;}else{hi=mid;}}
        lo
    };
    let mut start=0;let mut end=index.len();
    if matches!(mode,Mode::Merge|Mode::Adaptive) {
        start=bound(0,end,keys[0],false,&mut value);
        end=bound(start,end,*keys.last().unwrap(),true,&mut value);
        if matches!(mode,Mode::Adaptive) {
            // Estimate work from the covered range, rather than the full index size.
            let span=end-start;
            let log=usize::BITS-span.max(1).leading_zeros();
            mode=if span.saturating_add(keys.len()) <= keys.len().saturating_mul(log as usize) {Mode::Merge}else{Mode::Gallop};
        }
    }
    match mode {
        Mode::Point => {
            work.point=1;
            drop(value);
            for &key in keys {
                let lo=index.partition_point(|&row| {if TRACE {reads+=1;} r.value(row as usize,column)<key});
                let hi=index.partition_point(|&row| {if TRACE {reads+=1;} r.value(row as usize,column)<=key});
                out.extend(index[lo..hi].iter().map(|&r|r as usize));
            }
        }
        Mode::Merge => {work.merge=1;let mut at=start;for &key in keys {
            while at<end && value(at)<key {at+=1;}
            while at<end && value(at)==key {out.push(index[at] as usize);at+=1;}
        }}
        Mode::Gallop => {work.gallop=1;let mut at=start;for &key in keys {
            let origin=at;let mut hi=at;let mut step=1;
            while hi<end && value(hi)<key {at=hi+1;hi=origin.saturating_add(step).min(end);step=step.saturating_mul(2);}
            at=bound(at,hi.saturating_add(1).min(end),key,false,&mut value);
            while at<end && value(at)==key {out.push(index[at] as usize);at+=1;}
        }}
        Mode::Adaptive=>unreachable!(),
    }
    work.reads=reads;work.output=out.len();(out,work)
}
#[test]
#[ignore = "controlled index traversal experiments"]
fn lookup_shape_matrix() {
    use std::{hint::black_box,time::Instant};
    let mut relation=Relation::new(&["key","row"]);
    for i in 0..100_000 {relation.push(&[(i/2)*3,i]);}
    let cpu=||{let (u,s)=crate::code_rule_dsl::prof::thread_usage();u+s};
    let build=cpu();relation.index(0);let index_cpu=cpu()-build;
    let cases:Vec<(&str,Vec<i64>)>=vec![
        ("empty",vec![]),("one",vec![321]),("sparse",(0..32).map(|i|i*4500).collect()),
        ("clustered",(20_000..24_000).collect()),("dense",(0..150_000).step_by(3).collect()),
        ("absent",(0..150_000).step_by(3).map(|i|i+1).collect()),
        ("duplicate_unsorted",vec![300,3,300,-1,150_001]),
    ];
    let mut reports=Vec::new();
    for (name,keys) in cases {
        let expected:Vec<usize>=keys.iter().flat_map(|&key|relation.index(0).rows[relation.between(0,key,key)].iter().map(|&r|r as usize)).collect();
        for mode in [Mode::Point,Mode::Merge,Mode::Gallop,Mode::Adaptive] {
            let (got,work)=run::<true>(&relation,0,&keys,mode);assert_eq!(got,expected,"{name}: {mode:?}");
            let mut samples=Vec::new();
            for _ in 0..7 {let wall=Instant::now();let c=cpu();for _ in 0..20 {black_box(run::<false>(&relation,0,black_box(&keys),mode));}samples.push((cpu()-c,wall.elapsed().as_secs_f64()));}
            reports.push(serde_json::json!({"case":name,"mode":format!("{mode:?}"),"work":work,"samples":samples,"iterations":20,"parity":true}));
        }
    }
    let result=serde_json::json!({"indexBuildCpu":index_cpu,"rows":relation.len(),"reports":reports});
    std::fs::write(std::env::var("RERAIL_LOOKUP_REPORT").unwrap(),serde_json::to_string_pretty(&result).unwrap()).unwrap();
}
