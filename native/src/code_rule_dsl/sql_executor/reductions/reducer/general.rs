//! Isolated ordered and computed summaries over immutable native support roots.
//! SQL evaluates every projection and reduction; publication only changes storage.
use super::*;

pub(super) fn apply(r: &Reducer, w: &mut Work<'_>, delta: &Delta) -> Result<Option<(String,u64)>> {
    let store=w.versions.unwrap();
    let k=r.keys();
    let width=r.width();
    let cols=names("",k);
    let mut phase=phase::Phase::default();
    let mut incoming=vec![];
    for (table,sign,count) in [(&delta.plus,1,delta.inserted),(&delta.minus,-1,delta.deleted)] {
        if count==0 { continue; }
        let mut q=r.projection.clone();
        q.body.from[0].relation=source(table,&r.columns);
        incoming.push(format!("SELECT p.*,{sign} FROM ({}) p",q.render()));
    }
    let signed=phase.relation(width+1,&incoming.join(" UNION ALL "));
    let keys=phase.relation(k,&format!("SELECT DISTINCT {cols} FROM {signed}"));
    let values=if matches!(r.kind,Kind::Min | Kind::Max | Kind::Join) {
        let fields=names("",width);
        let weights=phase.relation(width+1,&format!("SELECT {fields},sum(c{width}) FROM {signed} GROUP BY {fields}"));
        let totals=phase.relation(width+1,&format!("SELECT {},rr_nonnegative_count(coalesce(v.n,0)+d.c{width}) FROM {weights} d LEFT JOIN {} v ON {}",names("d",width),quote(&r.values),equal("v","d",width)));
        let active=phase.view(width+1,&format!("SELECT * FROM {totals} WHERE c{width}>0"));
        phase.patch_computed(store,&r.values,&weights,&active)?;
        Some((weights,active))
    } else {None};
    let sum=if matches!(r.kind,Kind::Sum) {
        let rows=phase.view(k+3,&format!("SELECT {cols},c{k},c{width},NULL FROM {signed} UNION ALL SELECT {},g.value,1,g.high FROM {keys} k CROSS JOIN {} g WHERE {}",names("g",k),quote(&r.groups),equal("g","k",k)));
        let sums=phase.relation(k+2,&format!("SELECT {cols},rr_sum_low(c{k},c{},c{}),rr_sum_high(c{k},c{},c{}) FROM {rows} GROUP BY {cols}",k+1,k+2,k+1,k+2));
        Some(sums)
    } else {None};
    let value=match r.kind {
        Kind::Sum => {
            let sums=sum.as_ref().unwrap();
            format!("(SELECT c{k} FROM {sums} s WHERE {})",equal("s","d",k))
        }
        Kind::Min | Kind::Max => {
            let (changed,next)=values.as_ref().unwrap();
            combined::phase::extreme(r,changed,next,if matches!(r.kind,Kind::Min) {"min"} else {"max"})
        }
        Kind::Join => {
            let (changed,next)=values.as_ref().unwrap();
            // Correlate each branch before UNION. Materializing an overlay of
            // every untouched ordered group would turn sparse edits into scans.
            let overlay=format!("SELECT v.* FROM {} v WHERE {} AND ({}) NOT IN(SELECT {} FROM {changed}) UNION ALL SELECT n.* FROM {next} n WHERE {}",
                quote(&r.values),equal("v","d",k),names("v",width),names("",width),equal("n","d",k));
            format!("(SELECT {} FROM ({overlay}) rr_join_values)",r.join.as_ref().unwrap().render())
        }
        _ => unreachable!("Count and existence use their specialized transition"),
    };
    let high=sum.as_ref().map(|s|format!(",(SELECT c{} FROM {s} s WHERE {})",k+1,equal("s","d",k))).unwrap_or_default();
    let weights=phase.relation(k+1,&format!("SELECT {cols},sum(c{width}) FROM {signed} GROUP BY {cols}"));
    let group_width=k+2+usize::from(sum.is_some());
    let totals=phase.relation(group_width,&format!("SELECT {},rr_nonnegative_count(coalesce(g.n,0)+d.c{k}),{value}{high} FROM {weights} d LEFT JOIN {} g ON {}",names("d",k),quote(&r.groups),equal("g","d",k)));
    let differs=match r.kind {
        Kind::Sum => format!("coalesce(g.value,0) IS NOT n.c{} OR coalesce(g.high,0) IS NOT n.c{}",k+1,k+2),
        Kind::Join => format!("coalesce(g.value,{0}) IS NOT coalesce(n.c{1},{0})",Reducer::empty_join().render(),k+1),
        _ => format!("g.value IS NOT n.c{}",k+1),
    };
    let changed=phase.relation(k,&format!("SELECT {} FROM {totals} n LEFT JOIN {} g ON {} WHERE {differs}",names("n",k),quote(&r.groups),equal("g","n",k)));
    let active=phase.view(group_width,&format!("SELECT * FROM {totals} WHERE c{k}>0"));
    phase.patch_computed(store,&r.groups,&keys,&active)?;
    let touched=w.output_table(k)?;
    let output=w.output_table(k)?;
    phase.load_empty(store,&touched,&keys)?;
    phase.load_empty(store,&output,&changed)?;
    let _context=w.profile_scope(||json!({"stage":"isolated native summary transition","relation":r.relation,"summary_policy":"Ordered and computed summaries retain their original gate and SQL evaluation; immutable roots require no private DDL"}));
    let counts=phase.execute(w)?;
    let touched=counts[&touched].rows as u64;
    Ok((touched>0).then_some((output,touched)))
}
