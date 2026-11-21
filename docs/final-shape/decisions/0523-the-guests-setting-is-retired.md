# 0523. The `.guests` setting is retired

Status: decided (2026-11-20), by the user: "Retire it", of three choices for what `.guests off` means after 0431
and 0470 (Q325 item 3). Supersedes the `.guests` part of 0058 §1 and 0055 §5.

## Intro

A rule set used to say which embedded code it sees:

```
.guests off          // see files only, not the code inside their strings
```

Since 0431 and 0470 that is decided elsewhere:

- A string a rule parses is a **tape value**. It is never code to a rule that does not reach it through `parsed`.
- A **program** a file holds (an HTML file's `<script>`) is code only in a run of its own language.
- A run reads only its own language's files. It reads another language's files for their programs only when it says
  `.embedded on` (0470 step 2).

So `.guests off` had almost nothing left to hide: programs of the run's language that other files hold, and those
are read only under `.embedded on`. The setting is removed, and `.embedded` is the one control.

## Decision

1. `.guests on` and `.guests off` are refused when a program is lowered. The message names `.embedded`.
   `.scope embedded true | false`, the same setting under another name, is refused the same way.
2. No program makes `guest_policy("library", …)` facts. `guest_policy` keeps the `.on guest_error …` settings
   (`errors`, `errors:GRAMMAR`) only.
3. A program's shown documents are files and the run language's code guests, always (`hiding::bound` is
   `ANNOTATION_KEYS`). What it reads of other languages' files is `.embedded`'s.
4. The libraries' rules that read `!guest_policy("library", "off")` lose that literal. Python's string annotations
   are parsed for every program that includes `python/names.rr`.
5. The JSON rules' translator prints no `.guests off`, and the python-main pack's rules lose the line.

## Requirements (each with its test before the code)

- R1. `.guests on`, `.guests off` and `.scope embedded …` are refused, naming `.embedded`.
- R2. A program states no `guest_policy("library", …)` fact, and its shown bound is `ANNOTATION_KEYS`.
- R3. No library rule reads `guest_policy("library", …)`: Python's string annotations are parsed in every program
  that includes `python/names.rr`.
- R4. The translator prints no `.guests` line, and no pack rule has one.
- R5. Combined programs share one batch and see the same documents. A guest relation reaches a tape value in each.

## Consequences

- The 537 python-main rules include no library that read the setting, except `bandit-b302-marshal` and
  `bandit-b306-mktemp-q` (both include `python/names.rr`). Those two parse string annotations now, as every other
  program does. Their calls are found as before.
- A combined run no longer has programs that show different documents: its libraries can be held once more often
  (decision 0089).

## Tests

- R1, R2: `the_guests_setting_is_refused`.
- R3, R4: `no_library_or_pack_rule_reads_the_guests_setting`, and the translator's pack test (`json_rules`).
- R5: `combined_programs_see_the_same_documents` (was `combined_programs_see_guests_by_their_own_policy`).
- `parsed_strings_are_reached_through_parsed` loses its `.guests off` half; `copies_for_a_guest_in_the_runs_own_language_are_folded_into_their_originals`
  loses its "guests hidden" case.
