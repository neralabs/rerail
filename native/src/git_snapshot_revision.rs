//! Gix parses revision syntax. This delegate reads each object before following its links.

use gix::bstr::{BStr, ByteSlice};
use gix::error::{ErrorExt, message};
use gix::object::Kind;
use gix::revision::plumbing::spec::{self, parse::delegate};
use gix::{Object, ObjectId, Repository, Result};
use std::cell::RefCell;
use std::collections::HashSet;

use super::verified_object;

pub fn resolve(repo: &Repository, expression: &str) -> Result<ObjectId> {
    let mut revision = Revision {
        repo,
        reference: None,
        candidates: Vec::new(),
        read_failure: RefCell::new(None),
    };
    let parsed = spec::parse(expression.as_bytes().as_bstr(), &mut revision);
    // A checksum failure while narrowing an ambiguous prefix must not select a
    // different candidate. The parser can also wrap errors with syntax context.
    if let Some(failure) = revision.read_failure.into_inner() {
        return Err(failure);
    }
    parsed?;
    match revision.candidates.as_slice() {
        [id] => Ok(*id),
        [] => Err(message("No Git revision matched").raise()),
        _ => Err(message!("Ambiguous Git revision {expression:?}").raise()),
    }
}

struct Revision<'repo> {
    repo: &'repo Repository,
    reference: Option<gix::Reference<'repo>>,
    candidates: Vec<ObjectId>,
    read_failure: RefCell<Option<gix::Error>>,
}

impl<'repo> Revision<'repo> {
    fn anchor(&self) -> Result<Vec<ObjectId>> {
        if let ([], Some(reference)) = (self.candidates.as_slice(), &self.reference) {
            return Ok(vec![reference.clone().follow_to_object()?.detach()]);
        }
        Ok(self.candidates.clone())
    }

    fn read(&self, id: ObjectId) -> Result<Object<'repo>> {
        verified_object(self.repo, id).map_err(|error| {
            let detail = error.to_string();
            self.read_failure
                .borrow_mut()
                .get_or_insert_with(|| message!("{id}: {detail}").raise());
            message!("{id}: {detail}").raise()
        })
    }

    fn peel(&self, mut id: ObjectId, kind: Option<Kind>) -> Result<Object<'repo>> {
        loop {
            let object = self.read(id)?;
            if kind == Some(object.kind) {
                return Ok(object);
            }
            id = match object.kind {
                Kind::Tag => object.into_tag().target_id()?.detach(),
                Kind::Commit if kind == Some(Kind::Tree) => {
                    object.into_commit().tree_id()?.detach()
                }
                _ if kind.is_none() => return Ok(object),
                _ => return Err(message!("Git object {id} cannot be peeled to {kind:?}").raise()),
            };
        }
    }

    fn navigate(&mut self, step: impl Fn(&Self, ObjectId) -> Result<ObjectId>) -> Result<()> {
        let mut next = Vec::new();
        let mut rejected = None;
        for id in self.anchor()? {
            match step(self, id) {
                Ok(value) => next.push(value),
                Err(error) => rejected = Some(error),
            }
        }
        if next.is_empty() {
            return Err(rejected
                .unwrap_or_else(|| message("Git expression has no starting revision").raise()));
        }
        next.sort_unstable();
        next.dedup();
        self.candidates = next;
        Ok(())
    }

    fn path<'path>(&self, path: &'path BStr) -> Result<std::borrow::Cow<'path, BStr>> {
        // Only an explicit ./ or ../ is relative to the caller's worktree directory.
        if path.starts_with(b"./") || path.starts_with(b"../") {
            self.repo
                .prefix()?
                .ok_or_else(|| message("Relative Git paths require a worktree").raise())?;
            self.repo.normalize_path(path)
        } else {
            Ok(path.into())
        }
    }

    fn tree_path(&self, id: ObjectId, path: &BStr) -> Result<ObjectId> {
        let path = self.path(path)?;
        let mut tree = self.peel(id, Some(Kind::Tree))?.into_tree();
        if path.is_empty() {
            return Ok(tree.id);
        }
        let mut segments = path.split(|byte| *byte == b'/').peekable();
        while let Some(name) = segments.next() {
            let entry = tree.find_entry(name).ok_or_else(|| {
                message!("Git path {path:?} does not exist in tree {}", tree.id).raise()
            })?;
            let target = entry.id().detach();
            if segments.peek().is_none() {
                return Ok(target);
            }
            tree = self.read(target)?.try_into_tree()?;
        }
        unreachable!("an empty path returned the root")
    }

    fn matching_commit(
        &self,
        tips: Vec<ObjectId>,
        pattern: &BStr,
        negated: bool,
    ) -> Result<ObjectId> {
        let shallow = self.repo.shallow_commits()?;
        let mut seen = HashSet::new();
        let mut pending = gix::revwalk::PriorityQueue::new();
        let mut enqueue = |id, pending: &mut gix::revwalk::PriorityQueue<_, _>| -> Result<()> {
            if seen.insert(id) {
                let commit = self.read(id)?.try_into_commit()?;
                pending.insert(commit.committer()?.time()?.seconds, id);
            }
            Ok(())
        };
        for id in tips {
            enqueue(id, &mut pending)?;
        }
        while let Some(id) = pending.pop_value() {
            let commit = self.read(id)?.try_into_commit()?;
            if commit.message_raw_sloppy().contains_str(pattern) ^ negated {
                return Ok(commit.id);
            }
            if shallow
                .as_ref()
                .is_none_or(|commits| commits.binary_search(&commit.id).is_err())
            {
                for parent in commit.parent_ids() {
                    enqueue(parent.detach(), &mut pending)?;
                }
            }
        }
        Err(message!("No Git commit message matched {pattern:?}").raise())
    }
}

impl delegate::Revision for Revision<'_> {
    fn find_ref(&mut self, name: &BStr) -> Result<()> {
        self.reference = Some(self.repo.find_reference(name)?);
        self.candidates.clear();
        Ok(())
    }

    fn disambiguate_prefix(
        &mut self,
        prefix: gix::hash::Prefix,
        _: Option<delegate::PrefixHint<'_>>,
    ) -> Result<()> {
        let kind = self.repo.object_hash();
        let len = prefix.hex_len();
        if len < kind.len_in_hex()
            && let Some(reference) = self.repo.try_find_reference(&prefix.to_string())?
        {
            self.reference = Some(reference);
            return Ok(());
        }
        let mut padded = ObjectId::null(kind);
        let bytes = prefix
            .as_oid()
            .as_bytes()
            .get(..len.div_ceil(2))
            .expect("gix parsed the prefix");
        if bytes.len() > kind.len_in_bytes() {
            return Err(message!("Git object prefix {prefix} is too long for {kind}").raise());
        }
        padded.as_mut_slice()[..bytes.len()].copy_from_slice(bytes);
        let prefix = gix::hash::Prefix::new(&padded, len)?;
        let mut candidates = HashSet::new();
        self.repo
            .objects
            .lookup_prefix(prefix, Some(&mut candidates))?;
        if len == kind.len_in_hex() && padded == ObjectId::empty_tree(kind) {
            candidates.insert(padded);
        }
        if candidates.is_empty() {
            return Err(message!("No Git object matches prefix {prefix}").raise());
        }
        self.candidates = candidates.into_iter().collect();
        self.candidates.sort_unstable();
        Ok(())
    }

    fn reflog(&mut self, query: delegate::ReflogLookup) -> Result<()> {
        use delegate::ReflogLookup;
        let mut reference = match &self.reference {
            Some(reference) => reference.clone(),
            None => self
                .repo
                .head_ref()?
                .ok_or_else(|| message("No branch reflog for HEAD").raise())?,
        };
        if !reference.log_exists()? {
            reference.follow_to_object()?;
        }
        let mut log = reference.log_iter();
        let mut entries = log
            .rev()
            .map_err(ErrorExt::raise)?
            .ok_or_else(|| message("Git reference has no reflog").raise())?;
        let value = match query {
            ReflogLookup::Entry(index) => {
                entries.nth(index).transpose()?.map(|entry| entry.new_oid)
            }
            ReflogLookup::Date(time) => {
                let mut oldest = None;
                for entry in entries {
                    let entry = entry?;
                    let id = entry.new_oid;
                    if entry.signature.time.seconds <= time.seconds {
                        oldest = Some(id);
                        break;
                    }
                    oldest = Some(if entry.previous_oid.is_null() {
                        id
                    } else {
                        entry.previous_oid
                    });
                }
                oldest
            }
        }
        .ok_or_else(|| message("No Git reflog entry matches").raise())?;
        self.candidates = vec![value];
        Ok(())
    }

    fn nth_checked_out_branch(&mut self, number: usize) -> Result<()> {
        let mut log = self.repo.head()?.log_iter();
        let entries = log
            .rev()
            .map_err(ErrorExt::raise)?
            .ok_or_else(|| message("HEAD has no checkout history").raise())?;
        let name = entries
            .filter_map(std::result::Result::ok)
            .filter_map(|entry| {
                let from = entry.message.strip_prefix(b"checkout: moving from ")?;
                Some(from[..from.find(" to ")?].as_bstr().to_owned())
            })
            .nth(number.saturating_sub(1))
            .ok_or_else(|| message("No matching prior checkout").raise())?;
        if let Some(reference) = self.repo.try_find_reference(name.as_bstr())? {
            let target = reference.clone().follow_to_object()?.detach();
            self.candidates = vec![self.peel(target, None)?.id];
            self.reference = Some(reference);
            return Ok(());
        }
        self.candidates = vec![ObjectId::from_hex(name.as_ref())?];
        Ok(())
    }

    fn sibling_branch(&mut self, kind: delegate::SiblingBranch) -> Result<()> {
        let mut reference = match &self.reference {
            Some(reference) => reference.clone(),
            None => self
                .repo
                .head_ref()?
                .ok_or_else(|| message("HEAD has no branch").raise())?,
        };
        if reference.name() == "HEAD" {
            reference.follow_to_object()?;
        }
        let direction = match kind {
            delegate::SiblingBranch::Upstream => gix::remote::Direction::Fetch,
            delegate::SiblingBranch::Push => gix::remote::Direction::Push,
        };
        let tracking = reference
            .remote_tracking_ref_name(direction)
            .ok_or_else(|| message("No Git tracking branch is configured").raise())??;
        self.find_ref(tracking.as_ref().as_bstr())
    }
}

impl delegate::Kind for Revision<'_> {
    fn kind(&mut self, _: spec::Kind) -> Result<()> {
        Err(message("Git revision expressions must resolve to a single object").raise())
    }
}

impl delegate::Navigate for Revision<'_> {
    fn traverse(&mut self, traversal: delegate::Traversal) -> Result<()> {
        use delegate::Traversal;
        let shallow = self.repo.shallow_commits()?;
        self.navigate(|revision, id| {
            let mut commit = revision.peel(id, Some(Kind::Commit))?.into_commit();
            let parent = |commit: &gix::Commit<'_>, number| {
                if shallow
                    .as_ref()
                    .is_some_and(|commits| commits.binary_search(&commit.id).is_ok())
                {
                    return Err(message!("Shallow history ends at {}", commit.id).raise());
                }
                commit
                    .parent_ids()
                    .nth(number)
                    .map(gix::Id::detach)
                    .ok_or_else(|| {
                        message!("Git commit {} has no parent {}", commit.id, number + 1).raise()
                    })
            };
            match traversal {
                Traversal::NthParent(number) => parent(&commit, number - 1),
                Traversal::NthAncestor(steps) => {
                    for _ in 0..steps {
                        commit = revision.read(parent(&commit, 0)?)?.try_into_commit()?;
                    }
                    Ok(commit.id)
                }
            }
        })
    }

    fn peel_until(&mut self, kind: delegate::PeelTo<'_>) -> Result<()> {
        use delegate::PeelTo;
        self.navigate(|revision, id| match kind {
            PeelTo::ValidObject => Ok(revision.read(id)?.id),
            PeelTo::RecursiveTagObject => Ok(revision.peel(id, None)?.id),
            PeelTo::ObjectKind(kind) => Ok(revision.peel(id, Some(kind))?.id),
            PeelTo::Path(path) => revision.tree_path(id, path),
        })
    }

    fn find(&mut self, pattern: &BStr, negated: bool) -> Result<()> {
        if !self.anchor()?.is_empty() {
            return self.navigate(|revision, id| {
                let id = revision.peel(id, Some(Kind::Commit))?.id;
                revision.matching_commit(vec![id], pattern, negated)
            });
        }
        let references = self.repo.references()?;
        let mut tips = Vec::new();
        for reference in references.all()? {
            let id = reference?.follow_to_object()?.detach();
            let object = self.peel(id, None)?;
            if object.kind == Kind::Commit {
                tips.push(object.id);
            }
        }
        self.candidates = vec![self.matching_commit(tips, pattern, negated)?];
        Ok(())
    }

    fn index_lookup(&mut self, path: &BStr, stage: u8) -> Result<()> {
        let stage = match stage {
            0 => gix::index::entry::Stage::Unconflicted,
            1 => gix::index::entry::Stage::Base,
            2 => gix::index::entry::Stage::Ours,
            3 => gix::index::entry::Stage::Theirs,
            _ => return Err(message("Git index stage is out of range").raise()),
        };
        let path = self.path(path)?;
        let index = self.repo.index()?;
        let entry = index
            .entry_by_path_and_stage(&path, stage)
            .ok_or_else(|| message!("Git index has no entry for {path:?} at {stage:?}").raise())?;
        self.candidates = vec![entry.id];
        Ok(())
    }
}

impl spec::parse::Delegate for Revision<'_> {
    fn done(&mut self) -> Result<()> {
        self.candidates = self.anchor()?;
        if self.candidates.len() > 1
            && let Some(hint) = self.repo.config_snapshot().string("core.disambiguate")
            && let Some(hint) =
                gix::config::tree::Core::DISAMBIGUATE.try_into_object_kind_hint(hint)?
        {
            use gix::revision::spec::parse::ObjectKindHint;
            let mut preferred = Vec::new();
            for &id in &self.candidates {
                let matches = match hint {
                    ObjectKindHint::Commit => self.read(id)?.kind == Kind::Commit,
                    ObjectKindHint::Tree => self.read(id)?.kind == Kind::Tree,
                    ObjectKindHint::Blob => self.read(id)?.kind == Kind::Blob,
                    ObjectKindHint::Committish => self.peel(id, Some(Kind::Commit)).is_ok(),
                    ObjectKindHint::Treeish => self.peel(id, Some(Kind::Tree)).is_ok(),
                };
                if matches {
                    preferred.push(id);
                }
            }
            if !preferred.is_empty() {
                self.candidates = preferred;
            }
        }
        Ok(())
    }
}
