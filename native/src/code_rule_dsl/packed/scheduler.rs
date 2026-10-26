use super::metal_batch_abi::{
    GreedyEntryRecord, LinkRecord, MetalNode, PathRecord, QueryBucketRecord, QueryMemberRecord,
    QueuedStateRecord, RepeatRecord,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Admission {
    Accepted,
    Rejected,
    Capacity,
}

fn mix(hash: u32, value: u32) -> u32 {
    hash ^ value
        .wrapping_add(0x9e37_79b9)
        .wrapping_add(hash << 6)
        .wrapping_add(hash >> 2)
}

fn cursor_value(links: &[LinkRecord], head: u32, slot: u32) -> Option<u32> {
    let mut head = head;
    while head != 0 {
        let link = links.get(head.checked_sub(1)? as usize)?;
        if link.first == slot {
            return Some(link.second);
        }
        head = link.previous;
    }
    None
}

pub(super) fn cursor_equal(links: &[LinkRecord], left: u32, right: u32, slots: u32) -> bool {
    (0..slots).all(|slot| cursor_value(links, left, slot) == cursor_value(links, right, slot))
}

pub(super) fn repeat_equal(links: &[RepeatRecord], left: u32, right: u32) -> bool {
    let (mut left, mut right) = (left, right);
    while left != 0 && right != 0 {
        let Some(left_link) = links.get((left - 1) as usize) else {
            return false;
        };
        let Some(right_link) = links.get((right - 1) as usize) else {
            return false;
        };
        if left_link.value != right_link.value {
            return false;
        }
        left = left_link.previous;
        right = right_link.previous;
    }
    left == right
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CaptureIdentity {
    link: u32,
    label: u32,
    node: u32,
    start: u32,
    end: u32,
}

fn capture_before(left: CaptureIdentity, right: CaptureIdentity) -> bool {
    (left.start, core::cmp::Reverse(left.end), left.link)
        < (right.start, core::cmp::Reverse(right.end), right.link)
}

fn ordered_capture(
    links: &[LinkRecord],
    nodes: &[MetalNode],
    head: u32,
    ordinal: u32,
) -> Option<CaptureIdentity> {
    let mut selected = None;
    let mut selected_rank = u32::MAX;
    let mut candidate_head = head;
    while candidate_head != 0 {
        let candidate_link = links.get((candidate_head - 1) as usize)?;
        let node = nodes.get(candidate_link.second as usize)?;
        let candidate = CaptureIdentity {
            link: candidate_head,
            label: candidate_link.first,
            node: candidate_link.second,
            start: node.source_start,
            end: node.source_end,
        };
        let mut rank = 0_u32;
        let mut other_head = head;
        while other_head != 0 {
            let other_link = links.get((other_head - 1) as usize)?;
            let other_node = nodes.get(other_link.second as usize)?;
            let other = CaptureIdentity {
                link: other_head,
                label: other_link.first,
                node: other_link.second,
                start: other_node.source_start,
                end: other_node.source_end,
            };
            rank += u32::from(capture_before(other, candidate));
            other_head = other_link.previous;
        }
        if rank == ordinal && rank < selected_rank {
            selected = Some(candidate);
            selected_rank = rank;
        }
        candidate_head = candidate_link.previous;
    }
    selected
}

fn capture_count(links: &[LinkRecord], mut head: u32) -> Option<u32> {
    let mut count = 0_u32;
    while head != 0 {
        let link = links.get((head - 1) as usize)?;
        count = count.checked_add(1)?;
        head = link.previous;
    }
    Some(count)
}

pub(super) fn capture_containment(
    links: &[LinkRecord],
    nodes: &[MetalNode],
    left: u32,
    right: u32,
) -> Option<(bool, bool)> {
    let left_count = capture_count(links, left)?;
    let right_count = capture_count(links, right)?;
    let (mut left_at, mut right_at) = (0_u32, 0_u32);
    let (mut left_contains_right, mut right_contains_left) = (true, true);
    while left_at < left_count || right_at < right_count {
        let left_value = (left_at < left_count)
            .then(|| ordered_capture(links, nodes, left, left_at))
            .flatten();
        let right_value = (right_at < right_count)
            .then(|| ordered_capture(links, nodes, right, right_at))
            .flatten();
        match (left_value, right_value) {
            (Some(left_value), Some(right_value))
                if left_value.node == right_value.node && left_value.label == right_value.label =>
            {
                left_at += 1;
                right_at += 1;
            }
            (Some(left_value), Some(right_value)) => {
                let left_order = (left_value.start, core::cmp::Reverse(left_value.end));
                let right_order = (right_value.start, core::cmp::Reverse(right_value.end));
                match left_order.cmp(&right_order) {
                    core::cmp::Ordering::Less => {
                        right_contains_left = false;
                        left_at += 1;
                    }
                    core::cmp::Ordering::Greater => {
                        left_contains_right = false;
                        right_at += 1;
                    }
                    core::cmp::Ordering::Equal => {
                        left_contains_right = false;
                        right_contains_left = false;
                        left_at += 1;
                        right_at += 1;
                    }
                }
            }
            (Some(_), None) => {
                right_contains_left = false;
                break;
            }
            (None, Some(_)) => {
                left_contains_right = false;
                break;
            }
            (None, None) => break,
        }
    }
    Some((left_contains_right, right_contains_left))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum QueueInsert {
    Inserted,
    Duplicate,
    Capacity,
}

fn exact_captures(captures: &[LinkRecord], nodes: &[MetalNode], left: u32, right: u32) -> bool {
    capture_containment(captures, nodes, left, right) == Some((true, true))
}

fn queued_hash(path: PathRecord, sibling: u32) -> u32 {
    mix(
        mix(
            mix(
                mix(mix(path.pc, path.node), path.capture_hash),
                path.cursor_hash,
            ),
            path.repeat_hash,
        ),
        sibling,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn queue_insert(
    records: &mut [QueuedStateRecord],
    path: PathRecord,
    sibling: u32,
    captures: &[LinkRecord],
    cursors: &[LinkRecord],
    repeats: &[RepeatRecord],
    nodes: &[MetalNode],
    maximum_cursor_slot: u32,
) -> QueueInsert {
    if records.is_empty() {
        return QueueInsert::Capacity;
    }
    let hash = queued_hash(path, sibling);
    let mut slot = hash as usize % records.len();
    let mut available = None;
    for _ in 0..records.len() {
        let record = records[slot];
        if record.occupied == 0 {
            available.get_or_insert(slot);
            break;
        }
        if record.occupied == 2 {
            available.get_or_insert(slot);
        } else if record.hash == hash
            && record.pc == path.pc
            && record.node == path.node
            && record.sibling_requirement == sibling
            && exact_captures(captures, nodes, record.captures, path.captures)
            && cursor_equal(cursors, record.cursors, path.cursors, maximum_cursor_slot)
            && repeat_equal(repeats, record.repeats, path.repeats)
        {
            return QueueInsert::Duplicate;
        }
        slot = (slot + 1) % records.len();
    }
    let Some(slot) = available else {
        return QueueInsert::Capacity;
    };
    records[slot] = QueuedStateRecord {
        occupied: 1,
        hash,
        pc: path.pc,
        node: path.node,
        sibling_requirement: sibling,
        captures: path.captures,
        cursors: path.cursors,
        repeats: path.repeats,
    };
    QueueInsert::Inserted
}

#[allow(clippy::too_many_arguments)]
pub(super) fn queue_remove(
    records: &mut [QueuedStateRecord],
    path: PathRecord,
    sibling: u32,
    captures: &[LinkRecord],
    cursors: &[LinkRecord],
    repeats: &[RepeatRecord],
    nodes: &[MetalNode],
    maximum_cursor_slot: u32,
) -> bool {
    if records.is_empty() {
        return false;
    }
    let hash = queued_hash(path, sibling);
    let mut slot = hash as usize % records.len();
    for _ in 0..records.len() {
        let record = records[slot];
        if record.occupied == 0 {
            return false;
        }
        if record.occupied == 1
            && record.hash == hash
            && record.pc == path.pc
            && record.node == path.node
            && record.sibling_requirement == sibling
            && exact_captures(captures, nodes, record.captures, path.captures)
            && cursor_equal(cursors, record.cursors, path.cursors, maximum_cursor_slot)
            && repeat_equal(repeats, record.repeats, path.repeats)
        {
            records[slot].occupied = 2;
            return true;
        }
        slot = (slot + 1) % records.len();
    }
    false
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GreedySchedule {
    Inserted(u32),
    Replaced(u32),
    Capacity,
}

fn greedy_hash(path: PathRecord) -> u32 {
    mix(
        mix(
            mix(mix(path.pc, path.start), path.capture_hash),
            path.cursor_hash,
        ),
        path.repeat_hash,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn schedule_greedy(
    records: &mut [GreedyEntryRecord],
    path: PathRecord,
    captures: &[LinkRecord],
    cursors: &[LinkRecord],
    repeats: &[RepeatRecord],
    nodes: &[MetalNode],
    maximum_cursor_slot: u32,
) -> GreedySchedule {
    if records.is_empty() {
        return GreedySchedule::Capacity;
    }
    let hash = greedy_hash(path);
    let mut slot = hash as usize % records.len();
    let mut available = None;
    for _ in 0..records.len() {
        let record = records[slot];
        if record.occupied == 0 {
            available.get_or_insert(slot);
            break;
        }
        if record.occupied == 2 {
            available.get_or_insert(slot);
        } else if record.hash == hash
            && record.pc == path.pc
            && record.start == path.start
            && exact_captures(captures, nodes, record.captures, path.captures)
            && cursor_equal(cursors, record.cursors, path.cursors, maximum_cursor_slot)
            && repeat_equal(repeats, record.repeats, path.repeats)
        {
            records[slot].path = path;
            return GreedySchedule::Replaced(slot as u32);
        }
        slot = (slot + 1) % records.len();
    }
    let Some(slot) = available else {
        return GreedySchedule::Capacity;
    };
    records[slot] = GreedyEntryRecord {
        occupied: 1,
        hash,
        pc: path.pc,
        start: path.start,
        captures: path.captures,
        cursors: path.cursors,
        repeats: path.repeats,
        path,
    };
    GreedySchedule::Inserted(slot as u32)
}

pub(super) fn take_greedy(records: &mut [GreedyEntryRecord], slot: u32) -> Option<PathRecord> {
    let record = records.get_mut(slot as usize)?;
    if record.occupied != 1 {
        return None;
    }
    record.occupied = 2;
    Some(record.path)
}

pub(super) struct AdmissionTable<'a> {
    pub buckets: &'a mut [QueryBucketRecord],
    pub members: &'a mut [QueryMemberRecord],
    pub member_count: &'a mut u32,
    pub captures: &'a [LinkRecord],
    pub cursors: &'a [LinkRecord],
    pub repeats: &'a [RepeatRecord],
    pub nodes: &'a [MetalNode],
    pub maximum_cursor_slot: u32,
}

impl AdmissionTable<'_> {
    fn key_equal(&self, bucket: QueryBucketRecord, path: PathRecord, sibling: u32) -> bool {
        bucket.pc == path.pc
            && bucket.node == path.node
            && bucket.sibling_requirement == sibling
            && cursor_equal(
                self.cursors,
                bucket.cursors,
                path.cursors,
                self.maximum_cursor_slot,
            )
            && repeat_equal(self.repeats, bucket.repeats, path.repeats)
    }

    pub(super) fn admit(&mut self, path: PathRecord, sibling: u32) -> Admission {
        if self.buckets.is_empty() {
            return Admission::Capacity;
        }
        let hash = mix(
            mix(
                mix(mix(path.pc, path.node), path.cursor_hash),
                path.repeat_hash,
            ),
            sibling,
        );
        let mut slot = hash as usize % self.buckets.len();
        let mut found = None;
        for _ in 0..self.buckets.len() {
            let bucket = self.buckets[slot];
            if bucket.occupied == 0 {
                self.buckets[slot] = QueryBucketRecord {
                    occupied: 1,
                    hash,
                    pc: path.pc,
                    node: path.node,
                    sibling_requirement: sibling,
                    cursors: path.cursors,
                    repeats: path.repeats,
                    first_member: 0,
                };
                found = Some(slot);
                break;
            }
            if bucket.hash == hash && self.key_equal(bucket, path, sibling) {
                found = Some(slot);
                break;
            }
            slot = (slot + 1) % self.buckets.len();
        }
        let Some(slot) = found else {
            return Admission::Capacity;
        };
        let mut member = self.buckets[slot].first_member;
        while member != 0 {
            let Some(existing) = self.members.get((member - 1) as usize) else {
                return Admission::Capacity;
            };
            let Some((existing_contains_new, _)) =
                capture_containment(self.captures, self.nodes, existing.captures, path.captures)
            else {
                return Admission::Capacity;
            };
            if existing_contains_new {
                return Admission::Rejected;
            }
            member = existing.next_member;
        }
        let Ok(index) = usize::try_from(*self.member_count) else {
            return Admission::Capacity;
        };
        let Some(target) = self.members.get_mut(index) else {
            return Admission::Capacity;
        };
        *target = QueryMemberRecord {
            next_member: self.buckets[slot].first_member,
            captures: path.captures,
            capture_count: capture_count(self.captures, path.captures).unwrap_or(u32::MAX),
            capture_hash: path.capture_hash,
        };
        *self.member_count += 1;
        self.buckets[slot].first_member = *self.member_count;
        Admission::Accepted
    }
}
