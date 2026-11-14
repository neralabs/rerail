# Rerail patch

This directory contains `block` 0.1.6 from crates.io.

The patch makes two source changes:

- It replaces the uninhabited Objective-C class type with an opaque C-compatible structure.
- It adds the explicit `C` ABI to foreign functions.

These changes remove current and future Rust compiler warnings. They do not change the block ABI.
