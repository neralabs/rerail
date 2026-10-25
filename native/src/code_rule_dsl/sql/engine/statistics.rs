//! SQL owns statistics. Rust reads only the point lookups a plan decision requires.
use super::*;
#[derive(Clone, Default)]
pub(super) struct PlannerStats {
    pub kinds: BTreeMap<u16, u64>,
    pub source: Option<Arc<crate::content_store::ContentStore>>,
}
impl PlannerStats {
    pub fn read(
        source: Arc<crate::content_store::ContentStore>,
        kinds: &[u16],
    ) -> Result<Self, SqlError> {
        let values = source
            .search_kind_counts(kinds)
            .map_err(|e| SqlError::Sql(e.to_string()))?;
        Ok(Self {
            kinds: values.into_iter().collect(),
            source: Some(source),
        })
    }
    pub fn rows(&self, kinds: &[u16]) -> Option<u64> {
        kinds
            .iter()
            .try_fold(0u64, |sum, k| Some(sum.saturating_add(*self.kinds.get(k)?)))
    }
    pub fn refresh(&self) -> Result<Self, SqlError> {
        match &self.source {
            Some(source) => Self::read(
                source.clone(),
                &self.kinds.keys().copied().collect::<Vec<_>>(),
            ),
            None => Ok(self.clone()),
        }
    }
}
