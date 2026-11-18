# 0391. (was 0323) A guest's key is the store's, one per identity

Status: decided (2026-11-16), by the user (option 1 of four: "okay, let's try 1"). Builds on 0055 (guests and their
keys), 0058 §1 (the key ranges), 0052 §3 (a key a collision moved only misses), 0104 (no SQL inside the waves), 0306
(a run's small writes are one record). Q20 in the work queue.

## Intro

Every document has a key, and every node's id is its document's key shifted up 32 bits, plus the node's index. A
file's key comes from its path's hash. A guest (a type annotation, an embedded program, a parsed value) gets a key in
its own range: a hash of its host's path, its place in the host and its grammar.

Two guests can hash to the same key. Until now a collision moved the second guest to the next free key, but only
against the keys taken in the same chunk. Two hosts in different chunks, or one in a chunk and one a memo hit (whose
guests are never made), kept the same key. Their nodes then had the same ids.

On ep2 (1,208 files) one pair collided: `benchmarks/bench_versioning_git.py` and
`conform/l1100_rbac_schema/test_rbac_provider.py` both had an annotation document 1348786882. The held table folded
one file's row into the other's, and the kept statistics counted one row more than was held. The annotation range
holds 2^28 keys, so by the birthday bound a corpus with tens of thousands of guests has about one such pair, and the
9,010-file repository has dozens.

## Decision

1. **The store keeps each guest's key** in `guest_keys(identity, key, path)`: one row per guest identity (a SHA-256
   of its range and the parts its key was hashed from), `key` unique, `path` the host file (a value's first asker).
2. **Read once a process, before the first wave** (decision 0104): the whole table into memory, shared by the
   store's handles in the process, as its atoms are. About 60 bytes a guest.
3. **In the waves, the key is found in memory.** An identity the store knows has its key. A new one takes its hash's
   key, or the next past every key held, and is claimed in memory at once, so a later chunk, or a later run in the
   process, sees it.
4. **Written behind the run**: the run's new claims are an item of its record (0306), inserted by the writer.
5. **Two processes claiming one key.** A claim whose key or identity another process wrote first is not inserted, and
   the claim's host loses its memo entries: its next run misses, reads the registry again and takes a free key. This is
   0052 §3's rule for files: a key a collision moved only misses.
6. **Without a store** (a held memo, tests): the same registry in memory for the run, so the run's chunks still share
   it.

## Consequences

- A guest's key stays a function of what it is in a store, as 0055 says, and 0090's in-place maintenance holds again:
  a guest's rows are its host's entry's alone.
- Every run reads the registry once a process: about 25,000 rows on ep2. Rows of guests no file has any more stay. A
  row is about 60 bytes, and a range holds 2^28 keys. Collecting them waits for a measured need.
- A store made before this decision has guests in its memo and frames that the registry does not hold. A new engine
  build's memo is cold, so its runs make every guest again and claim it.
- §5 drops the losing host's entries in the store. A process that admitted them into its memo snapshot before the
  waves still holds them until it reads the memo again, so only a later run in that same process could hit one.
  Another process reads the store.
- Built in 51e67e692: `content_store/guest_keys.rs`. The registry lives on the store's `Atoms`, so a store created
  again at its path has a registry of its own. On ep2's fresh store the cold run claims 27,073 keys, and the edits
  claim none, so they write nothing.
- Tests:
  - `guest_keys::tests::a_guests_key_is_its_identitys_in_every_process_and_a_collision_probes_past_it` (store).
  - `guests::tests::two_chunks_guests_that_hash_alike_take_two_keys` (engine) fails without §1–3.
  - `guest_keys::tests::a_claim_another_process_wrote_first_drops_its_hosts_entries` (store), §5.
