//! Test-only physical ownership inventory, with no admission or query execution.
use super::{Instruction, Program};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
impl Program {
    pub(in crate::code_rule_dsl::sql::relational::outcomes) fn inventory(&self) -> Value {
        let mut owners = vec![BTreeSet::new(); self.code.len()];
        fn walk(p: &Program, pc: usize, owner: usize, owners: &mut [BTreeSet<usize>]) {
            if !owners[pc].insert(owner) {
                return;
            }
            let children = match &p.code[pc].op {
                Instruction::Boolean { children, .. } => children.clone(),
                Instruction::Not(p) => vec![*p],
                Instruction::Search { body, stop, .. } => {
                    std::iter::once(*body).chain(*stop).collect()
                }
                Instruction::Binding {
                    children,
                    occurrence,
                    ..
                } => children
                    .iter()
                    .map(|(_, pc)| *pc)
                    .chain(*occurrence)
                    .collect(),
                _ => vec![],
            };
            for pc in children {
                walk(p, pc, owner, owners);
            }
        }
        walk(self, self.entry, usize::MAX, &mut owners);
        for (id, f) in self.functions.iter().enumerate() {
            walk(self, f.entry, id, &mut owners);
        }
        let mut counts = BTreeMap::<&str, usize>::new();
        let nodes=self.code.iter().enumerate().map(|(pc,n)|{
            let (op,args)=match &n.op {
                Instruction::Eval(region)=>("eval",json!({"region":region})),Instruction::Call(f)=>("call",json!({"function":f})),
                Instruction::Boolean{all,children}=>(if *all{"all"}else{"any"},json!({"children":children})),
                Instruction::Not(child)=>("not",json!({"child":child})),
                Instruction::Search{axis,end,field,body,stop,bounds,filter}=>("search",json!({"axis":format!("{axis:?}"),"end":end,"field":field,"body":body,"stop":stop,"bounds":bounds,"filterKinds":filter.as_ref().map(|f|&f.kinds),"filterText":filter.as_ref().map(|f|&f.text)})),
                Instruction::Binding{children,occurrence,..}=>("binding",json!({"children":children,"occurrence":occurrence})),
            };
            *counts.entry(op).or_default()+=1;
            json!({"pc":pc,"op":op,"args":args,"owners":owners[pc].iter().map(|id|if *id==usize::MAX{"root".into()}else{format!("function{id}")}).collect::<Vec<_>>(),"mentions":n.mentions,"writes":n.writes,"nodeWrites":n.node_writes})
        }).collect::<Vec<_>>();
        json!({"opcodeCounts":counts,"nodes":nodes,"functions":self.functions.iter().enumerate().map(|(id,f)|json!({"id":id,"entry":f.entry,"memo":f.memo,"mentions":f.mentions,"writes":f.writes})).collect::<Vec<_>>()})
    }
}
