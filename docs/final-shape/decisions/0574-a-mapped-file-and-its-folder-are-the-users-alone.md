# 0574: A mapped file and its folder are the user's alone

**Status:** accepted (2026-11-21)

**The user (2026-11-21):** "make sure that all files that are memory mapped are marked read-only and owned by the
user, and that we fail if anything is group or world writable, and that rerail will fail to launch if their
containing folder is world or group writable or executable".

Asked how a file mapped for writing fits "read-only", the user chose: a file mapped for writing is made `0600` and
checked to be the user's alone; frozen or sealed, it is marked `0400`; a read-only mapping of a frozen file
requires no write bit. Every mapping checks the file is the user's, and fails on any group or world write bit.

## Why

The engine reads its blocks in place (decision 0353): a mapped file's bytes are the engine's data, read with no
check, by the CPU and the GPU. Whoever can write the file, or replace it in its folder, writes the engine's
memory. So the file and the folder it is in are the user's alone.

## The rules

`rerail_blocks::mapped` holds them, and every mapping goes through it (Unix; Windows has no such modes):

- **A folder** holding a mapped file is the user's, with no group or world write or search bit (`0700` at most:
  `0o033` refused). A folder the engine makes is made `0700` (`make_folder`, each missing folder above it too).
  One it is given that is not is refused, never repaired.
- **A file** mapped is the user's, with no group or world write bit (`0o022` refused, for reading and for
  writing).
  - Mapped for writing (an open block, decision 0401; a deferrable slab, 0295; a cost store's blocks): made `0600`
    (`open_write`).
  - Frozen or sealed: marked `0400` (`mark_frozen`: `Block::freeze`, `Atoms::freeze`, a deferrable slab's frozen
    state, an output image as it is put). A frozen file opened with its owner's write bit still on is marked so as
    it is opened (`open_read(path, true)`): the bit is the user's own, on the user's own file.
  - A writer that writes a frozen file of the user's again (a deferrable slab written back) puts its owner's write
    bit back first.
- **A refusal** is an error of kind `PermissionDenied` that names the path, its owner and its mode.
- **SQLite's databases** are mapped too (`mmap_size`): their folder is checked as a store is opened
  (`private_files::prepare`), and a database file another user owns, or with group or world write, is refused
  (`keep_private`, which still takes group and world read away).
- **The shipped bundle** (`rerail-library.images`, beside the executable, in a folder that is `0755` by any
  install) is read into memory, not mapped (`rr::images::map`): it is the install's, not the user's.

## At launch

`rerail-engine` refuses to start when a folder its mapped files are in is shared
(`tools::launch_checks`, before anything else runs): the existing folders of the tape cache
(`RERAIL_TAPE_CACHE`), the cost store, the image store (`RERAIL_IMAGE_STORE`), the atoms and pack folders, and the
temporary stores' folders. It prints `refusing to launch: …` with the folder and its mode, and exits with a
failure. A folder not made yet is made `0700` when it is first needed.

So a store folder made by hand with the umask's mode (`mkdir` gives `0755` under a umask of `022`) refuses the
launch until it is `chmod 700`. Scripts that make one say `mkdir -m 700`.

## Tests

- `mapped::tests::a_shared_folder_is_refused`, `mapped_files_are_the_users_and_frozen_ones_read_only`.
- `tools::launch_tests::a_shared_store_folder_refuses_the_launch`.
- The engine's tests run with a umask of `077` (`lib.rs`, `private_test_folders`): a fixture's folder
  (`tempfile::tempdir`, `create_dir_all`) is made with the umask's mode, and the farm's hosts' `002` made them
  `775`, which the rules refuse. The test harness's long-lived folders made before the rules have group and world
  bits taken away once a process (`test_store::harness_folder`).
