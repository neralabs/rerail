# 0283: A finding's message holes are typed columns; the writer composes the message

Status: built (2026-11-15), in the merge of final-rerail-shape-phased-facts into the hub. Chosen by the user at the
merge ("typed holes, writer composes").

## Intro

A finding is a row the run writes out. Its message ("`{why}` is used at line `{at.line}`") is made of two things:
the words of the template, and the values that fill its placeholders (holes). Two decisions met at this merge:

- Decision 0225 §3 (the hub): the words are not part of the program. The rule carries only the holes' values, the
  template is kept beside the program as a `.message` setting, and the digests that key preparation and the memo
  read the template's skeleton (holes, no words). So a message's edit re-evaluates nothing: the writer makes each
  row's message from the template and the values. The hub's rule was one relation, `rr_diagnostic`, a JSON text per
  row, with `"message":[v1,v2,…]` in it.
- Decision 0249 §6 (phased-facts): a finding is not a JSON text. Each `.violation` has its own relation,
  `rr_diagnostic_<digest>`, of typed columns (`findings::HEAD`, then the tag, then each parameter's name and value),
  so no reader parses a text. Its rule computed the message whole, words included, in the `message` column.

Each side alone breaks the other: 0249's rule puts the words back into the program (a message's edit is a change of
the rules again), and 0225 §3's JSON row is the text 0249 removed.

## Decision

The holes are typed columns at the end of the finding's relation. The writer composes the message and writes the row
without them.

    // The lowered rule (findings.rs, `diagnostic_rule`):
    //   HEAD (policy, clause, code, path, line, at, message = "", tagged), [tag],
    //   name1, value1, …, hole0, hole1, …            // `findings::HOLE`, a text each
    // The writer (corpus.rs, `compose_messages`), for each `is_diagnostic` relation:
    //   k = the decl's `hole*` columns; template by (policy, clause), else ("", clause);
    //   message = message_of(template, holes), else holes.concat();
    //   row = cells[..w-k] with column 6 (`MESSAGE_COLUMN`) the message.

- The rule leaves `message` empty and keeps the `.message` setting (0225 §3); the keys read the skeleton
  (`rr::message_masked`), as before.
- The image the writer makes has 0249's columns only: a reader sees `message` filled and no hole columns.
- A finding's relations are never reused from a session's unchanged outputs (`is_diagnostic`), since their messages
  come from the templates of this run, as `rr_diagnostic` was not.

## Consequences

- No JSON in the findings path. 0225 §3's `composed_message` and its JSON parsing are gone.
- A message's edit still leaves the program, its preparation and the memo as they were.
- The hole columns exist only between the rule and the writer: a test that reads a finding's relation through the
  program's own run (not the written image) sees them.

## Tests

`rr::tests::findings_keep_messages_tags_parameters_and_witnesses`, `doubled_braces_in_a_message_are_text`,
`source_span_producers_retain_rows_through_local_planning`: the message read from the written image.
