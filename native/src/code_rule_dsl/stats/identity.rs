//! Plan identities (05 Phase 5 row 3), from ner-1562-scale-test-the-re-railer's
//! `query/plan/identity.rs`: domain-separated SHA-256 over tagged, length-prefixed fields, so two
//! identities are equal only for equal inputs of the same domain. `compiled_plans` is keyed by a
//! [`CompiledPlanId`] (`stats::plans`), built from the plan's semantic identities and the
//! prefilter's candidate-graph identity, so the interpreted and JIT bindings of a plan share it
//! (D10).
use sha2::{Digest as _, Sha256};
use std::fmt;
use std::marker::PhantomData;

const IDENTITY_MAGIC: &[u8] = b"rerail.query-plan.identity\0";
const IDENTITY_CONTRACT_VERSION: u16 = 1;

pub(in crate::code_rule_dsl) trait IdentityDomain {
    const TAG: u8;
    const NAME: &'static str;
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::code_rule_dsl) struct ScopeRecipeDomain;

impl IdentityDomain for ScopeRecipeDomain {
    const TAG: u8 = 1;
    const NAME: &'static str = "scope-recipe";
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::code_rule_dsl) struct ScopeInstanceDomain;

impl IdentityDomain for ScopeInstanceDomain {
    const TAG: u8 = 2;
    const NAME: &'static str = "scope-instance";
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::code_rule_dsl) struct UtilityDefinitionDomain;

impl IdentityDomain for UtilityDefinitionDomain {
    const TAG: u8 = 11;
    const NAME: &'static str = "utility-definition";
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::code_rule_dsl) struct RelationDomain;

#[cfg(test)]
impl IdentityDomain for RelationDomain {
    const TAG: u8 = 3;
    const NAME: &'static str = "relation";
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::code_rule_dsl) struct ClausePlanDomain;

impl IdentityDomain for ClausePlanDomain {
    const TAG: u8 = 4;
    const NAME: &'static str = "clause-plan";
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::code_rule_dsl) struct EffectProfileDomain;

impl IdentityDomain for EffectProfileDomain {
    const TAG: u8 = 5;
    const NAME: &'static str = "effect-profile";
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::code_rule_dsl) struct DocumentMapGenerationDomain;

impl IdentityDomain for DocumentMapGenerationDomain {
    const TAG: u8 = 6;
    const NAME: &'static str = "document-map-generation";
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::code_rule_dsl) struct SnapshotDomain;

impl IdentityDomain for SnapshotDomain {
    const TAG: u8 = 8;
    const NAME: &'static str = "snapshot";
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::code_rule_dsl) struct DocumentDomain;

impl IdentityDomain for DocumentDomain {
    const TAG: u8 = 9;
    const NAME: &'static str = "document";
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::code_rule_dsl) struct CompiledPlanDomain;

impl IdentityDomain for CompiledPlanDomain {
    const TAG: u8 = 17;
    const NAME: &'static str = "compiled-plan";
}

pub(in crate::code_rule_dsl) type ScopeRecipeId = SemanticId<ScopeRecipeDomain>;
pub(in crate::code_rule_dsl) type ScopeInstanceId = SemanticId<ScopeInstanceDomain>;
#[cfg(test)]
pub(in crate::code_rule_dsl) type RelationId = SemanticId<RelationDomain>;
pub(in crate::code_rule_dsl) type ClausePlanId = SemanticId<ClausePlanDomain>;
pub(in crate::code_rule_dsl) type EffectProfileId = SemanticId<EffectProfileDomain>;
pub(in crate::code_rule_dsl) type DocumentMapGenerationId = SemanticId<DocumentMapGenerationDomain>;
pub(in crate::code_rule_dsl) type SnapshotId = SemanticId<SnapshotDomain>;
pub(in crate::code_rule_dsl) type DocumentId = SemanticId<DocumentDomain>;
pub(in crate::code_rule_dsl) type CompiledPlanId = SemanticId<CompiledPlanDomain>;
pub(in crate::code_rule_dsl) type UtilityDefinitionId = SemanticId<UtilityDefinitionDomain>;

pub(in crate::code_rule_dsl) fn utility_definition_id(
    origin_digest: &[u8; 32],
) -> UtilityDefinitionId {
    UtilityDefinitionId::builder().bytes(origin_digest).finish()
}

#[cfg(test)]
macro_rules! identity_domain {
    ($domain:ident, $identity:ident, $tag:literal, $name:literal) => {
        #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
        pub(in crate::code_rule_dsl) struct $domain;

        impl IdentityDomain for $domain {
            const TAG: u8 = $tag;
            const NAME: &'static str = $name;
        }

        pub(in crate::code_rule_dsl) type $identity = SemanticId<$domain>;
    };
}

#[cfg(test)]
identity_domain!(
    LanguageContractDomain,
    LanguageContractId,
    10,
    "language-contract"
);
#[cfg(test)]
identity_domain!(UtilityDemandDomain, UtilityDemandId, 12, "utility-demand");
#[cfg(test)]
identity_domain!(FrameDomain, FrameId, 13, "frame");
#[cfg(test)]
identity_domain!(ExclusionDomain, ExclusionId, 14, "exclusion");
#[cfg(test)]
identity_domain!(
    SemanticContextDomain,
    SemanticContextId,
    15,
    "semantic-context"
);
#[cfg(test)]
identity_domain!(LogicalFillDomain, LogicalFillId, 16, "logical-fill");

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::code_rule_dsl) struct SemanticId<D> {
    digest: [u8; 32],
    domain: PhantomData<D>,
}

impl<D: IdentityDomain> SemanticId<D> {
    pub(in crate::code_rule_dsl) fn builder() -> IdentityBuilder<D> {
        IdentityBuilder::new()
    }

    pub(in crate::code_rule_dsl) fn as_bytes(&self) -> &[u8; 32] {
        &self.digest
    }
}

impl<D: IdentityDomain> fmt::Display for SemanticId<D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(D::NAME)?;
        formatter.write_str(":")?;
        for byte in self.digest {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl<D: IdentityDomain> fmt::Debug for SemanticId<D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

pub(in crate::code_rule_dsl) struct IdentityBuilder<D> {
    digest: Sha256,
    domain: PhantomData<D>,
}

impl<D: IdentityDomain> IdentityBuilder<D> {
    fn new() -> Self {
        let mut digest = Sha256::new();
        digest.update(IDENTITY_MAGIC);
        digest.update(IDENTITY_CONTRACT_VERSION.to_be_bytes());
        digest.update([D::TAG]);
        Self {
            digest,
            domain: PhantomData,
        }
    }

    fn field(mut self, tag: u8, value: &[u8]) -> Self {
        self.digest.update([tag]);
        self.digest.update(
            u64::try_from(value.len())
                .expect("an identity field length fits in u64")
                .to_be_bytes(),
        );
        self.digest.update(value);
        self
    }

    pub(in crate::code_rule_dsl) fn bytes(self, value: &[u8]) -> Self {
        self.field(1, value)
    }

    pub(in crate::code_rule_dsl) fn text(self, value: &str) -> Self {
        self.field(2, value.as_bytes())
    }

    pub(in crate::code_rule_dsl) fn u16(self, value: u16) -> Self {
        self.field(3, &value.to_be_bytes())
    }

    pub(in crate::code_rule_dsl) fn u32(self, value: u32) -> Self {
        self.field(4, &value.to_be_bytes())
    }

    pub(in crate::code_rule_dsl) fn u64(self, value: u64) -> Self {
        self.field(5, &value.to_be_bytes())
    }

    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn boolean(self, value: bool) -> Self {
        self.field(6, &[u8::from(value)])
    }

    pub(in crate::code_rule_dsl) fn identity<I: IdentityDomain>(self, value: SemanticId<I>) -> Self {
        self.field(7, value.as_bytes())
    }

    pub(in crate::code_rule_dsl) fn finish(self) -> SemanticId<D> {
        SemanticId {
            digest: self.digest.finalize().into(),
            domain: PhantomData,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_separate_domains_field_kinds_and_boundaries() {
        let plan = CompiledPlanId::builder().text("ab").text("c").finish();
        assert_eq!(plan, CompiledPlanId::builder().text("ab").text("c").finish());
        assert_ne!(plan, CompiledPlanId::builder().text("a").text("bc").finish(), "fields are length-prefixed");
        assert_ne!(plan, CompiledPlanId::builder().bytes(b"ab").text("c").finish(), "fields are typed");
        let clause = ClausePlanId::builder().text("ab").text("c").finish();
        assert_ne!(plan.as_bytes(), clause.as_bytes(), "domains are separated");
        assert!(plan.to_string().starts_with("compiled-plan:"));
        let nested = CompiledPlanId::builder().identity(clause).u32(1).finish();
        assert_ne!(nested, CompiledPlanId::builder().identity(clause).u32(2).finish());
    }
}
