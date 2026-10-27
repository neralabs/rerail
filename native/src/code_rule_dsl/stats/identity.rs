use sha2::{Digest as _, Sha256};
use std::fmt;
use std::marker::PhantomData;

const IDENTITY_MAGIC: &[u8] = b"rerail.query-plan.identity\0";
const IDENTITY_CONTRACT_VERSION: u16 = 1;

pub(in crate::code_rule_dsl::query) trait IdentityDomain {
    const TAG: u8;
    const NAME: &'static str;
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct ScopeRecipeDomain;

impl IdentityDomain for ScopeRecipeDomain {
    const TAG: u8 = 1;
    const NAME: &'static str = "scope-recipe";
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct ScopeInstanceDomain;

impl IdentityDomain for ScopeInstanceDomain {
    const TAG: u8 = 2;
    const NAME: &'static str = "scope-instance";
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::code_rule_dsl::query) struct UtilityDefinitionDomain;

impl IdentityDomain for UtilityDefinitionDomain {
    const TAG: u8 = 11;
    const NAME: &'static str = "utility-definition";
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct RelationDomain;

#[cfg(test)]
impl IdentityDomain for RelationDomain {
    const TAG: u8 = 3;
    const NAME: &'static str = "relation";
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct ClausePlanDomain;

impl IdentityDomain for ClausePlanDomain {
    const TAG: u8 = 4;
    const NAME: &'static str = "clause-plan";
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct EffectProfileDomain;

impl IdentityDomain for EffectProfileDomain {
    const TAG: u8 = 5;
    const NAME: &'static str = "effect-profile";
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct DocumentMapGenerationDomain;

impl IdentityDomain for DocumentMapGenerationDomain {
    const TAG: u8 = 6;
    const NAME: &'static str = "document-map-generation";
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct SnapshotDomain;

impl IdentityDomain for SnapshotDomain {
    const TAG: u8 = 8;
    const NAME: &'static str = "snapshot";
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct DocumentDomain;

impl IdentityDomain for DocumentDomain {
    const TAG: u8 = 9;
    const NAME: &'static str = "document";
}

pub(super) type ScopeRecipeId = SemanticId<ScopeRecipeDomain>;
pub(super) type ScopeInstanceId = SemanticId<ScopeInstanceDomain>;
#[cfg(test)]
pub(super) type RelationId = SemanticId<RelationDomain>;
pub(super) type ClausePlanId = SemanticId<ClausePlanDomain>;
pub(super) type EffectProfileId = SemanticId<EffectProfileDomain>;
pub(super) type DocumentMapGenerationId = SemanticId<DocumentMapGenerationDomain>;
pub(super) type SnapshotId = SemanticId<SnapshotDomain>;
pub(super) type DocumentId = SemanticId<DocumentDomain>;
pub(in crate::code_rule_dsl::query) type UtilityDefinitionId = SemanticId<UtilityDefinitionDomain>;

pub(in crate::code_rule_dsl::query) fn utility_definition_id(
    origin_digest: &[u8; 32],
) -> UtilityDefinitionId {
    UtilityDefinitionId::builder().bytes(origin_digest).finish()
}

#[cfg(test)]
macro_rules! identity_domain {
    ($domain:ident, $identity:ident, $tag:literal, $name:literal) => {
        #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
        pub(super) struct $domain;

        impl IdentityDomain for $domain {
            const TAG: u8 = $tag;
            const NAME: &'static str = $name;
        }

        pub(super) type $identity = SemanticId<$domain>;
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
pub(in crate::code_rule_dsl::query) struct SemanticId<D> {
    digest: [u8; 32],
    domain: PhantomData<D>,
}

impl<D: IdentityDomain> SemanticId<D> {
    pub(super) fn builder() -> IdentityBuilder<D> {
        IdentityBuilder::new()
    }

    pub(super) fn as_bytes(&self) -> &[u8; 32] {
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

pub(super) struct IdentityBuilder<D> {
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

    pub(super) fn bytes(self, value: &[u8]) -> Self {
        self.field(1, value)
    }

    pub(super) fn text(self, value: &str) -> Self {
        self.field(2, value.as_bytes())
    }

    pub(super) fn u16(self, value: u16) -> Self {
        self.field(3, &value.to_be_bytes())
    }

    pub(super) fn u32(self, value: u32) -> Self {
        self.field(4, &value.to_be_bytes())
    }

    pub(super) fn u64(self, value: u64) -> Self {
        self.field(5, &value.to_be_bytes())
    }

    #[cfg(test)]
    pub(super) fn boolean(self, value: bool) -> Self {
        self.field(6, &[u8::from(value)])
    }

    pub(super) fn identity<I: IdentityDomain>(self, value: SemanticId<I>) -> Self {
        self.field(7, value.as_bytes())
    }

    pub(super) fn finish(self) -> SemanticId<D> {
        SemanticId {
            digest: self.digest.finalize().into(),
            domain: PhantomData,
        }
    }
}
