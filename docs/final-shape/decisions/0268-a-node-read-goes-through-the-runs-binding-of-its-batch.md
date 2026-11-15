# 0268: A node's text or value is read through the run's binding of its batch

Status: built (2026-11-15), in the merge of Typing-support-final-shape into the hub. Chosen by the user at the merge:
one mechanism for every node read, the binding's.

## Intro

Three functions read a node's text or value from the batch of documents the run parsed, not from a stored row:

- `@rr_node_text(n)`: a node's text past `text`'s inline bound (decision 0109's long-text twins).
- `@rr_node_text_any(n)`: a node's text at any length, which answers `node_text` on request (decision 0174).
- `@rr_node_value(n)`: a node's value as W reads it, which answers `value` on request (decision 0174).

A node number names its document by a key, and two runs can number their documents alike. So a read has to find
the batch of its own run. Before the merge, each side made that so in its own way:

- The hub kept a map from document key to batch, one per run on its thread (`TEXTS`, `own_texts`), and carried it
  to the run's jobs (`run_state`).
- Typing-support-final-shape (a20ea5b48) dropped the map. `rr_long_text_node`, the relation every long-text twin
  joins, is bound as `syntax::LongTexts`, which holds its batch. The native engine's `func` step takes that binding
  from its connection when it compiles the rule, and SQL's function is registered per connection and reads it.
  That side knew only `@rr_node_text`.

## Decision

All three are node reads (`syntax::NODE_READS`), and each goes through the run's binding of `rr_long_text_node`.

    // The func step, compiled (setfix.rs):
    let texts = if NODE_READS.contains(&name) {
        // The binding on the rule's own connection: its batch is the run's.
        Some(bound_source(conn, LONG_TEXT_NODES).ok_or_else(|| ...)?)
    } else { None };

`LongTexts` holds the batch and its documents by key, and reads one of three ways (`LongTexts::of`): `long_text`,
`text_any`, `value_any`. `functions::node_read` is the one entry point for the native step and for SQL's functions.
`functions::eval` gives no value for a node read, because it has no run.

The batch binds `rr_long_text_node` with every batch's relations, so a program that reads a node's text on
request finds the binding even where it has no long-text twin. A rule compiled on a connection with no binding is
an error, not an empty text.

The map, `own_texts` and its carry in `run_state` are gone.

## Consequences

- A read never reaches another run's batch, nor another chunk's: a node of a document outside the bound batch
  reads none. Before, the map could return a document of an earlier chunk if its batch was still alive. That was
  never counted on: the cross part's reads of other files' texts are open item O-cross-file-answers.
- No thread-local state to carry to jobs for texts.
