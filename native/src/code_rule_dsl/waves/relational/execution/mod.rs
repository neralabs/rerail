//! An explicit job owns immutable SQL facts; each caller owns its logical membership.
use super::{CompiledCodeRuleDsl, json::invalid, matcher::Corpus, matcher::CorpusSlot};
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
            owner: Mutex::new(Some(Arc::new(Corpus::default()))),
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
        let _domain = CorpusSlot::install(Arc::new(owner.fork()));
        compiled.evaluate(snapshot, mode)
    }

    #[napi]
    pub fn create_workspace(&self) -> Result<super::workspace::CodeRuleDslWorkspace> {
        let owner = self
            .owner
            .lock()
            .clone()
            .ok_or_else(|| invalid("execution is closed"))?;
        Ok(super::workspace::CodeRuleDslWorkspace::with_corpus(
            Arc::new(owner.fork()),
        ))
    }

    #[napi]
    pub fn close(&self) {
        self.owner.lock().take();
    }
}
