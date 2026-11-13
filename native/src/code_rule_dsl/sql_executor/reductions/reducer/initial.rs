//! One summary layout and SQL definition for direct and composed initialization.
use super::*;

pub struct Root {
    pub name: String,
    pub definition: String,
    pub query: String,
    pub width: usize,
}

pub fn roots(r: &Reducer, source: Option<&str>) -> Vec<Root> {
    let k = r.keys();
    let cols = names("", k);
    let value = format!("c{k}");
    let source=source.map(quote).unwrap_or_else(||format!("({})",r.projection.render()));
    let (suffix, reduction, width, values) = if let Some(shared) = &r.combined {
        let sum = if shared.layout.sum {
            format!("rr_sum_low({value},1,NULL),rr_sum_high({value},1,NULL)")
        } else { "0,0".into() };
        let min = if shared.layout.min {format!("min({value})")} else {"0".into()};
        let max = if shared.layout.max {format!("max({value})")} else {"0".into()};
        (",high INTEGER NOT NULL,minimum INTEGER NOT NULL,maximum INTEGER NOT NULL",
            format!("{sum},{min},{max}"), k+5, shared.layout.min || shared.layout.max)
    } else {
        let reduction = match r.kind {
            Kind::Count | Kind::Matches(_) => "count(*)".into(),
            Kind::Sum => format!("rr_sum_low({value},1,NULL),rr_sum_high({value},1,NULL)"),
            Kind::Min => format!("min({value})"),
            Kind::Max => format!("max({value})"),
            Kind::Join => r.join.as_ref().unwrap().render(),
        };
        let wide = matches!(r.kind,Kind::Sum);
        (if wide {",high INTEGER NOT NULL"} else {""}, reduction, k+2+usize::from(wide),
            matches!(r.kind,Kind::Min | Kind::Max | Kind::Join))
    };
    let mut roots = vec![Root {
        name:r.groups.clone(),
        definition:format!("{},n INTEGER NOT NULL CHECK(n>0),value INTEGER NOT NULL{suffix},PRIMARY KEY({cols})", schema(k)),
        query:format!("SELECT {cols},count(*),{reduction} FROM {source} rr_join_values GROUP BY {cols}"),
        width,
    }];
    if values {
        let width=r.width();
        let fields=names("",width);
        roots.push(Root {
            name:r.values.clone(),
            definition:format!("{},n INTEGER NOT NULL CHECK(n>0),PRIMARY KEY({fields})",schema(width)),
            query:format!("SELECT {fields},count(*) FROM {source} GROUP BY {fields}"),
            width:width+1,
        });
    }
    roots
}

pub fn initialize(
    r: &Reducer,
    db: &rusqlite::Connection,
    objects: &mut Vec<(String,bool)>,
    storage: Option<&versions::Store>,
    phase_publication: bool,
) -> Result<()> {
    if !r.shared_owner() || objects.iter().any(|(name,_)|name==&r.groups) { return Ok(()); }
    let readonly=phase_publication && storage.is_some();
    for root in roots(r,None) {
        owned_phase(db,objects,&root.name,&root.definition,storage,readonly)?;
        insert(db,&root.name,&root.query,storage)?;
    }
    Ok(())
}
