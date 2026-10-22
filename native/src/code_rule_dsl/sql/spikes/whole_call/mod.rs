//! Whole-call relational compilation experiments. No addon switch or production alternate path.
//! The adapter retains the existing semantic lowerings, but replaces physical frame steps with
//! immutable typed relation references. Unsupported stateful constructs fail explicitly.
mod adapter;
mod amplification;
mod buffers;
mod b023;
mod composition;
mod crossfile;
mod direct;
mod each;
mod graph;
mod homogeneous;
mod inputs;
mod integer_keys;
mod lex;
mod physical;
mod profile;
mod recursive;
mod recursive_machine;
mod recursive_machine_tests;
mod run;
mod scan;
mod shapes;
mod terminal_set;
mod tests;
mod workload;
