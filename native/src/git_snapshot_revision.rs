//! Gix parses revision syntax. This delegate reads each object before following its links.

use gix::bstr::{BStr, ByteSlice};
use gix::error::{ErrorExt, message};
use gix::object::Kind;
use gix::revision::plumbing::spec::{self, parse::delegate};
use gix::{Object, ObjectId, Repository, Result};
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::Path;

use super::verified_object;

#[path = "git_snapshot_message_pattern.rs"]
mod message_pattern;
use message_pattern::MessagePattern;

#[derive(Debug)]
struct LookupError {
    reason: &'static str,
    detail: String,
}

impl std::fmt::Display for LookupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for LookupError {}

fn lookup_error(reason: &'static str, detail: impl std::fmt::Display) -> gix::Error {
    gix::Error::from_error(LookupError { reason, detail: detail.to_string() })
}

pub fn failure_reason(error: &gix::Error) -> &'static str {
    error.downcast_any_ref::<LookupError>().map_or("invalid-revision", |cause| cause.reason)
}

pub fn failure_detail(error: &gix::Error) -> String {
    error.downcast_any_ref::<LookupError>().map_or_else(|| error.probable_cause().to_string(), |cause| cause.detail.clone())
}

pub fn resolve(repo: &Repository, expression: &str, path_base: Option<&Path>) -> Result<ObjectId> {
    Ok(selection(repo, expression, path_base)?.0)
}

pub(super) fn selection(repo: &Repository, expression: &str, path_base: Option<&Path>) -> Result<(ObjectId, bool)> {
    let mut revision = Revision {
        anchor_name: expression.split(['^', '~', ':']).next().unwrap_or_default().split("@{").next().unwrap_or_default(),
        selects_path: false,
        repo,
        path_base,
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
        [id] => Ok((*id, revision.selects_path)),
        [] => Err(lookup_error("missing-revision", "No Git revision matched")),
        _ => Err(lookup_error("ambiguous-revision", format_args!("Ambiguous Git revision {expression:?}"))),
    }
}

struct Revision<'repo> {
    anchor_name: &'repo str,
    selects_path: bool,
    repo: &'repo Repository,
    path_base: Option<&'repo Path>,
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
        rerail_prof::count("git.revision_object_reads", 1);
        verified_object(self.repo, id).map_err(|error| {
            let reason = match error.kind() {
                std::io::ErrorKind::NotFound => "missing-object",
                std::io::ErrorKind::InvalidData => "corrupt-object",
                _ => "unreadable-object",
            };
            self.read_failure
                .borrow_mut()
                .get_or_insert_with(|| lookup_error(reason, format_args!("Git object {id}: {error}")));
            lookup_error(reason, format_args!("Git object {id}: {error}"))
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
                _ => return Err(lookup_error("wrong-object-kind",
                    format_args!("Git object {id} is {}, expected {}", object.kind, kind.expect("requested object kind")))),
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
            use std::borrow::Cow;
            let base = self.path_base
                .ok_or_else(|| message("Relative Git paths require a worktree").raise())?;
            let relative = gix::path::from_bstr(Cow::Borrowed(path))?;
            let full = gix::path::normalize_and_clean(Cow::Owned(base.join(relative)), Path::new(""))
                .ok_or_else(|| message!("Git path {path:?} leaves the repository").raise())?;
            Ok(gix::path::to_unix_separators_on_windows(gix::path::into_bstr(full)?))
        } else {
            Ok(path.into())
        }
    }

    fn tree_path(&self, id: ObjectId, path: &BStr) -> Result<ObjectId> {
        let directory = path.ends_with(b"/") || path.ends_with(b"/.");
        let path = self.path(path)?;
        let mut tree = self.peel(id, Some(Kind::Tree))?.into_tree();
        if path.is_empty() {
            return Ok(tree.id);
        }
        let mut segments = path.split(|byte| *byte == b'/').peekable();
        while let Some(name) = segments.next() {
            let tree_id = tree.id;
            let entries = super::decode_tree(tree).map_err(|error| {
                lookup_error("malformed-object", format_args!("Git tree {tree_id}: {error}"))
            })?;
            let index = entries.binary_search_by(|entry| entry.name.as_slice().cmp(name)).map_err(|_| {
                lookup_error("missing-path", format_args!("Git path {path:?} does not exist in tree {tree_id}"))
            })?;
            let entry = &entries[index];
            let target = entry.target;
            if segments.peek().is_none() {
                if directory && !entry.is_tree() {
                    return Err(lookup_error("wrong-object-kind", format_args!("Git path {path:?} is not a directory")));
                }
                return Ok(target);
            }
            if !entry.is_tree() {
                return Err(lookup_error("wrong-object-kind", format_args!("Git path component in {path:?} is not a directory")));
            }
            if segments.clone().eq([&b""[..]]) {
                return Ok(target);
            }
            let object = self.read(target)?;
            if object.kind != Kind::Tree {
                return Err(lookup_error("wrong-object-kind", format_args!("Git object {target} is {}, expected tree", object.kind)));
            }
            tree = object.into_tree();
        }
        unreachable!("an empty path returned the root")
    }

    fn matching_commit(
        &self,
        tips: Vec<gix::Commit<'repo>>,
        pattern: &BStr,
        matcher: &MessagePattern,
        negated: bool,
    ) -> Result<ObjectId> {
        let shallow = self.repo.shallow_commits()?;
        let mut seen = HashSet::new();
        let mut pending = gix::revwalk::PriorityQueue::new();
        let mut order = 0;
        let mut enqueue = |commit: gix::Commit<'repo>, pending: &mut gix::revwalk::PriorityQueue<_, _>| -> Result<()> {
            pending.insert((commit.committer()?.time()?.seconds, std::cmp::Reverse(order)), commit);
            order += 1;
            Ok(())
        };
        for commit in tips {
            if seen.insert(commit.id) { enqueue(commit, &mut pending)?; }
        }
        while let Some(commit) = pending.pop_value() {
            if matcher.matches(commit.message_raw_sloppy()).map_err(|error| lookup_error("invalid-revision", error))? ^ negated {
                return Ok(commit.id);
            }
            if shallow
                .as_ref()
                .is_none_or(|commits| commits.binary_search(&commit.id).is_err())
            {
                for parent in commit.parent_ids() {
                    let id = parent.detach();
                    if seen.insert(id) { enqueue(self.read(id)?.try_into_commit()?, &mut pending)?; }
                }
            }
        }
        Err(lookup_error("missing-revision", format_args!("No Git commit message matched {pattern:?}")))
    }
}

impl delegate::Revision for Revision<'_> {
    fn find_ref(&mut self, name: &BStr) -> Result<()> {
        self.reference = Some(self.repo.try_find_reference(name)?.ok_or_else(|| {
            lookup_error("missing-revision", format_args!("No Git reference matches {name:?}"))
        })?);
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
        if self.anchor_name.contains('-') {
            if let Some(reference) = self.repo.try_find_reference(self.anchor_name)? {
                self.reference = Some(reference);
                return Ok(());
            }
            let valid = self.anchor_name.rsplit_once("-g").is_some_and(|(base, hex)| {
                hex.eq_ignore_ascii_case(&prefix.to_string()) && base.rsplit_once('-').is_some_and(|(name, distance)| {
                    !name.is_empty() && !distance.is_empty() && distance.bytes().all(|b| b.is_ascii_digit())
                })
            });
            if !valid { return Err(lookup_error("missing-revision", self.anchor_name)); }
        }
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
            return Err(lookup_error("missing-revision", format_args!("No Git object matches prefix {prefix}")));
        }
        self.candidates = candidates.into_iter().collect();
        self.candidates.sort_unstable();
        Ok(())
    }

    fn reflog(&mut self, query: delegate::ReflogLookup) -> Result<()> {
        use delegate::ReflogLookup;
        let mut reference = match &self.reference {
            Some(reference) => reference.clone(),
            None => match self.repo.head_ref()? {
                Some(branch) => branch,
                None => self.repo.find_reference("HEAD")?,
            },
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
        self.selects_path |= matches!(kind, PeelTo::Path(_));
        self.navigate(|revision, id| match kind {
            PeelTo::ValidObject => Ok(revision.read(id)?.id),
            PeelTo::RecursiveTagObject => Ok(revision.peel(id, None)?.id),
            PeelTo::ObjectKind(kind) => Ok(revision.peel(id, Some(kind))?.id),
            PeelTo::Path(path) => revision.tree_path(id, path),
        })
    }

    fn find(&mut self, pattern: &BStr, negated: bool) -> Result<()> {
        if pattern.is_empty() && !negated {
            return self.navigate(|revision, id| Ok(revision.peel(id, Some(Kind::Commit))?.id));
        }
        let matcher = MessagePattern::new(pattern).map_err(|error| {
            lookup_error("invalid-revision", format_args!("Invalid Git message pattern {pattern:?}: {error}"))
        })?;
        if !self.anchor()?.is_empty() {
            return self.navigate(|revision, id| {
                let commit = revision.peel(id, Some(Kind::Commit))?.into_commit();
                revision.matching_commit(vec![commit], pattern, &matcher, negated)
            });
        }
        let references = self.repo.references()?;
        let mut tips = Vec::new();
        let mut first_error = None;
        let mut tip_ids = HashSet::new();
        for reference in references.all()? {
            let mut reference = reference?;
            let name = reference.name().to_owned();
            let result = reference.follow_to_object().and_then(|id| {
                let id = id.detach();
                if tip_ids.insert(id) { self.peel(id, None).map(Some) } else { Ok(None) }
            });
            let object = match result {
                Ok(Some(object)) => object,
                Ok(None) => continue,
                Err(error) => {
                    let error = self.read_failure.borrow_mut().take().unwrap_or(error);
                    eprintln!("Git message search skipped {name}: {}", failure_detail(&error));
                    first_error.get_or_insert(error);
                    continue;
                }
            };
            if object.kind == Kind::Commit {
                tips.push(object.into_commit());
            }
        }
        match self.repo.head().and_then(|head| head.id().map(gix::Id::detach)
            .filter(|id| tip_ids.insert(*id)).map(|id| self.peel(id, None)).transpose()) {
            Ok(Some(object)) if object.kind == Kind::Commit => tips.push(object.into_commit()),
            Ok(_) => {}
            Err(error) => {
                let error = self.read_failure.borrow_mut().take().unwrap_or(error);
                eprintln!("Git message search skipped HEAD: {}", failure_detail(&error));
                first_error.get_or_insert(error);
            }
        }
        let found = self.matching_commit(tips, pattern, &matcher, negated).map_err(|error| {
            if failure_reason(&error) == "missing-revision" { first_error.unwrap_or(error) } else { error }
        })?;
        self.candidates = vec![found];
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
