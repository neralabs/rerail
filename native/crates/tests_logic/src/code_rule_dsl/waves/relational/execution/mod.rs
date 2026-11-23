//! An explicit job owns immutable SQL facts; each caller owns its logical membership.
use crate::code_rule_dsl::{CompiledCodeRuleDsl, json::invalid};
// W's matcher has no corpus (D's `matcher::Corpus`, `CorpusSlot`): the owner is an open flag (O19).
type Corpus = ();
use napi::Result;
use napi_derive::napi;
use parking_lot::Mutex;
use std::sync::Arc;

#[napi]
pub struct CodeRuleDslExecution {
    owner: Mutex<Option<Arc<Corpus>>>,
}

impl Default for CodeRuleDslExecution {
    fn default() -> Self {
        Self::new()
    }
}

#[napi]
impl CodeRuleDslExecution {
    #[napi(constructor)]
    pub fn new() -> Self {
        Self {
            owner: Mutex::new(Some(Arc::new(()))),
        }
    }

    #[napi]
    pub fn evaluate(
        &self,
        compiled: &CompiledCodeRuleDsl,
        snapshot: String,
        mode: String,
    ) -> Result<String> {
        let owner = self
            .owner
            .lock()
            .clone()
            .ok_or_else(|| invalid("execution is closed"))?;
        let _ = owner;
        compiled.evaluate(snapshot, mode)
    }

    #[napi]
    pub fn create_workspace(&self) -> Result<crate::code_rule_dsl::workspace::api::CodeRuleDslWorkspace> {
        let owner = self
            .owner
            .lock()
            .clone()
            .ok_or_else(|| invalid("execution is closed"))?;
        let _ = owner;
        Ok(crate::code_rule_dsl::workspace::api::CodeRuleDslWorkspace::new())
    }

    #[napi]
    pub fn close(&self) {
        self.owner.lock().take();
    }
}
