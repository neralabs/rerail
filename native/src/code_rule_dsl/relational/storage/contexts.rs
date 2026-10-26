//! Stable SQLite backing with exact data bindings. Scope changes are data writes, never DDL.
use super::{Connection,Result,quote};
use std::collections::{BTreeMap,BTreeSet};
const BINDINGS:&str="__execution_bindings";
const KEY:&str="__execution_key";
pub(super) fn managed(name:&str)->bool {name.starts_with("__context_")||name.starts_with("__stage___context_")}
fn backing(name:&str)->String {format!("__rows_{name}")}
#[derive(Default)]
pub(super) struct Contexts {
    initialized:bool,
    owner:usize,
    active:BTreeMap<String,usize>,
    layouts:BTreeMap<String,Layout>,
    data:BTreeMap<(String,usize),Data>,
    owners:BTreeMap<usize,BTreeSet<(String,usize)>>,
}
struct Layout {definition:String,indexes:BTreeMap<String,String>,analyzed:bool}
#[derive(Default)]
struct Data {dirty:bool,ready:bool}
impl Contexts {
    pub fn bind(&mut self,db:&Connection,owner:usize,bindings:BTreeMap<String,usize>)->Result<()> {
        if !self.initialized {
            db.execute_batch(&format!("CREATE TEMP TABLE {BINDINGS}(slot TEXT PRIMARY KEY,binding INTEGER NOT NULL) WITHOUT ROWID"))?;
            self.initialized=true;
        }
        if self.active!=bindings {
            db.execute(&format!("DELETE FROM {BINDINGS}"),[])?;
            let rows=bindings.iter().collect::<Vec<_>>();
            for chunk in rows.chunks(256) {
                let values=std::iter::repeat_n("(?,?)",chunk.len()).collect::<Vec<_>>().join(",");
                let args=chunk.iter().flat_map(|(slot,id)|[rusqlite::types::Value::Text((*slot).clone()),rusqlite::types::Value::Integer(**id as i64)]);
                db.execute(&format!("INSERT INTO {BINDINGS} VALUES {values}"),rusqlite::params_from_iter(args))?;
            }
            self.active=bindings;
        }
        self.owner=owner;
        Ok(())
    }
    pub fn contains(&self,name:&str)->bool {self.layouts.contains_key(name)}
    pub fn declare(&mut self,db:&Connection,name:&str,definition:&str)->Result<()> {
        if self.layouts.get(name).is_some_and(|l|l.definition==definition) {return Ok(());}
        if self.contains(name) {self.discard(db,name)?;}
        let body=definition.split_once('(').ok_or(rusqlite::Error::InvalidQuery)?.1;
        db.execute_batch(&format!("CREATE TEMP TABLE {}({KEY} INTEGER NOT NULL,{body}",quote(&backing(name))))?;
        let columns={let stmt=db.prepare(&format!("SELECT * FROM {} LIMIT 0",quote(&backing(name))))?;stmt.column_names().into_iter().skip(1).map(quote).collect::<Vec<_>>().join(",")};
        let literal=name.replace('\'',"''");
        db.execute_batch(&format!("CREATE TEMP VIEW {} AS SELECT {columns} FROM {} WHERE {KEY}=(SELECT binding FROM {BINDINGS} WHERE slot='{literal}')",quote(name),quote(&backing(name))))?;
        db.execute_batch(&format!("CREATE INDEX {} ON {}({KEY})",quote(&format!("__binding_{name}")),quote(&backing(name))))?;
        self.layouts.insert(name.into(),Layout{definition:definition.into(),indexes:BTreeMap::new(),analyzed:false});
        Ok(())
    }
    fn key(&self,name:&str)->Option<(String,usize)> {self.active.get(name).map(|&id|(name.to_owned(),id))}
    pub fn target(&self,name:&str)->Option<(String,Option<i64>)> {
        if !managed(name) {return None;}
        self.active.get(name).map(|&id|(backing(name),Some(id as i64)))
    }
    pub fn index(&mut self,db:&Connection,slot:&str,name:&str,sql:&str)->Result<()> {
        let layout=self.layouts.get_mut(slot).ok_or(rusqlite::Error::InvalidQuery)?;
        if layout.indexes.get(name).is_some_and(|s|s==sql) {return Ok(());}
        if layout.indexes.contains_key(name) {db.execute_batch(&format!("DROP INDEX {}",quote(name)))?;}
        let fields=sql.split_once('(').ok_or(rusqlite::Error::InvalidQuery)?.1;
        db.execute_batch(&format!("CREATE INDEX {} ON {}({KEY},{fields}",quote(name),quote(&backing(slot))))?;
        layout.indexes.insert(name.into(),sql.into());
        Ok(())
    }
    pub fn analyze(&mut self,db:&Connection,name:&str,rows:usize)->Result<()> {
        let Some(layout)=self.layouts.get_mut(name) else {return Err(rusqlite::Error::InvalidQuery)};
        // A context contains multiple bindings. Reanalyzing its complete backing for
        // each binding creates quadratic work. The first populated index supplies
        // an initial estimate; completed publication batches refresh it explicitly.
        if rows>0&&!layout.analyzed {
            db.execute_batch(&format!("ANALYZE temp.{}",quote(&backing(name))))?;
            layout.analyzed=true;
        }
        Ok(())
    }
    pub fn written(&mut self,name:&str,rows:bool) {
        if let Some(key)=self.key(name) {
            self.data.entry(key.clone()).or_default().dirty|=rows;
            self.owners.entry(self.owner).or_default().insert(key);
        }
    }
    pub fn ready(&self,name:&str)->bool {self.key(name).and_then(|key|self.data.get(&key)).is_some_and(|d|d.ready)}
    pub fn publish(&mut self,name:&str) {if let Some(key)=self.key(name) {self.data.entry(key).or_default().ready=true;}}
    pub fn retire(&mut self,db:&Connection,name:&str)->Result<bool> {
        let Some(key)=self.key(name) else {return Ok(false)};
        if self.data.get(&key).is_some_and(|d|!d.ready) {self.remove(db,&key)?;}
        Ok(self.contains(name))
    }
    fn remove(&mut self,db:&Connection,key:&(String,usize))->Result<()> {
        if self.data.get(key).is_some_and(|d|d.dirty) {
            db.execute(&format!("DELETE FROM {} WHERE {KEY}=?1",quote(&backing(&key.0))),[key.1 as i64])?;
        }
        self.data.remove(key);
        Ok(())
    }
    pub fn clear_owner(&mut self,db:&Connection,owner:usize)->Result<()> {
        for key in self.owners.remove(&owner).unwrap_or_default() {
            if self.data.get(&key).is_some_and(|d|!d.ready) {self.remove(db,&key)?;}
        }
        Ok(())
    }
    pub fn discard(&mut self,db:&Connection,name:&str)->Result<()> {
        db.execute_batch(&format!("DROP VIEW IF EXISTS {}; DROP TABLE IF EXISTS {}",quote(name),quote(&backing(name))))?;
        self.layouts.remove(name);
        self.data.retain(|(slot,_),_|slot!=name);
        Ok(())
    }
    pub fn bytes(&self)->usize {
        self.layouts.iter().map(|(n,l)|n.len()+l.definition.len()+l.indexes.iter().map(|(n,q)|n.len()+q.len()+64).sum::<usize>()+128).sum::<usize>()
            +self.data.len()*128+self.active.iter().map(|(n,_)|n.len()+48).sum::<usize>()
    }
}
