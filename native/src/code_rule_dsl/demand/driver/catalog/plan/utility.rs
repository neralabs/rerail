use super::identity::{
    DocumentId, ExclusionId, FrameId, LanguageContractId, RelationId, SemanticContextId,
    UtilityDefinitionId, UtilityDemandId,
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FrameFieldKind {
    Node,
    Text,
    Sequence,
}

impl FrameFieldKind {
    fn tag(self) -> u16 {
        match self {
            Self::Node => 1,
            Self::Text => 2,
            Self::Sequence => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct UtilityEffects {
    three_valued: bool,
    reads_file_state: bool,
    reads_clause_state: bool,
    writes_captures: bool,
    ordered: bool,
}

impl UtilityEffects {
    pub(super) fn pure() -> Self {
        Self {
            three_valued: false,
            reads_file_state: false,
            reads_clause_state: false,
            writes_captures: false,
            ordered: false,
        }
    }

    pub(super) fn with_clause_state(mut self) -> Self {
        self.reads_clause_state = true;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct UtilityDefinition {
    id: UtilityDefinitionId,
    body: RelationId,
    language: LanguageContractId,
    parameters: Vec<FrameFieldKind>,
    effects: UtilityEffects,
}

impl UtilityDefinition {
    pub(super) fn new(
        body: RelationId,
        language: LanguageContractId,
        parameters: Vec<FrameFieldKind>,
        effects: UtilityEffects,
    ) -> Self {
        let mut identity = UtilityDefinitionId::builder()
            .identity(body)
            .identity(language)
            .u64(parameters.len() as u64);
        for parameter in &parameters {
            identity = identity.u16(parameter.tag());
        }
        identity = identity
            .boolean(effects.three_valued)
            .boolean(effects.reads_file_state)
            .boolean(effects.reads_clause_state)
            .boolean(effects.writes_captures)
            .boolean(effects.ordered);
        Self {
            id: identity.finish(),
            body,
            language,
            parameters,
            effects,
        }
    }

    pub(super) fn id(&self) -> UtilityDefinitionId {
        self.id
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct UtilityIdentityCollision {
    id: UtilityDefinitionId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CatalogEntry {
    definition: UtilityDefinition,
    provenance: Vec<String>,
}

#[derive(Default)]
pub(super) struct UtilityCatalog {
    definitions: BTreeMap<UtilityDefinitionId, CatalogEntry>,
}

impl UtilityCatalog {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn intern(
        &mut self,
        provenance: impl Into<String>,
        definition: UtilityDefinition,
    ) -> Result<UtilityDefinitionId, UtilityIdentityCollision> {
        let id = definition.id();
        let provenance = provenance.into();
        match self.definitions.get_mut(&id) {
            Some(entry) if entry.definition != definition => Err(UtilityIdentityCollision { id }),
            Some(entry) => {
                match entry.provenance.binary_search(&provenance) {
                    Ok(_) => {}
                    Err(position) => entry.provenance.insert(position, provenance),
                }
                Ok(id)
            }
            None => {
                self.definitions.insert(
                    id,
                    CatalogEntry {
                        definition,
                        provenance: vec![provenance],
                    },
                );
                Ok(id)
            }
        }
    }

    pub(super) fn definition_count(&self) -> usize {
        self.definitions.len()
    }

    pub(super) fn provenance(&self, id: UtilityDefinitionId) -> &[String] {
        self.definitions
            .get(&id)
            .map_or(&[], |entry| entry.provenance.as_slice())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct UtilityDemand {
    id: UtilityDemandId,
    definition: UtilityDefinitionId,
    document: DocumentId,
    node_ordinal: u32,
    frame: FrameId,
    exclusion: Option<ExclusionId>,
    context: SemanticContextId,
    three_valued: bool,
}

impl UtilityDemand {
    pub(super) fn new(
        definition: UtilityDefinitionId,
        document: DocumentId,
        node_ordinal: u32,
        frame: FrameId,
        exclusion: Option<ExclusionId>,
        context: SemanticContextId,
        three_valued: bool,
    ) -> Self {
        let mut identity = UtilityDemandId::builder()
            .identity(definition)
            .identity(document)
            .u32(node_ordinal)
            .identity(frame)
            .boolean(exclusion.is_some());
        if let Some(exclusion) = exclusion {
            identity = identity.identity(exclusion);
        }
        let id = identity.identity(context).boolean(three_valued).finish();
        Self {
            id,
            definition,
            document,
            node_ordinal,
            frame,
            exclusion,
            context,
            three_valued,
        }
    }

    pub(super) fn id(&self) -> UtilityDemandId {
        self.id
    }
}
