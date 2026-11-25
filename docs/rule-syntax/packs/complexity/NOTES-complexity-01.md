# complexity-01: complexity-cyclomatic-rust, complexity-cyclomatic-vue

Both rules are `same` as their JSON originals on every corpus the suite compares (examples, fixtures,
rerail-typescript). Ground truth: lizard 1.24 (`uvx lizard`) on the fixtures.

## complexity-cyclomatic-rust

Baseline: fixtures `lizard/rust/branches.rs:138` original-only.

```rust
pub fn let_chain_over_limit(first: Option<u32>, second: Option<u32>, ready: bool) -> u32 {
    if let Some(a) = first && let Some(b) = second && ready && a > b { return a - b; }
    if let Some(a) = first && a > 10 { return a; }
    0
}
```

lizard: CCN 7 (1 + 2 `if` + 4 let-chain `&&`); the original reports it. The translation was wrong: it scored 3.
`lib/complexity/rust.rerail` reads a let chain's `&&` with `op_at(chain, i, "&&")`, and the engine gives
`op_at` no rows for a `let_chain` (checked with a probe rule: `op(chain, "&&")` and `token(chain, i, "&&", _, _)`
hold, `op_at` does not), though core.rerail documents `op_at(chain, 0, "&&")`. The rule adds a `cc_decision` row
from `token` (the index is the token's place among the chain's children), guarded by `!op_at(n, _, "&&")` so it
never counts twice once op_at has rows.

**Library change proposed** (`lib/complexity/rust.rerail`, not edited here): replace the let-chain row by
`cc_decision(n, i) :- kind(n, "let_chain"), token(n, i, "&&", _, _).` and drop the rule's row.
**Engine gap:** `op_at` has no rows for a Rust `let_chain`, against core.rerail's own example.

## complexity-cyclomatic-vue

Baseline: nothing reported (examples `Grade.vue:4`, fixtures `options.vue:8, :37`, `setup.vue:7, :32` all
original-only). lizard agrees with the original on the four fixtures (CCN 8, 8, 8, 9); the example's `grade` has
the same body as setup.vue's (lizard 8). The translation was wrong.

Cause: the rule read the scripts through `vscript` (`lib/vue/component.rerail`), which needs core `parsed` rows
for a `<script>` block. The engine gives none for Vue (rrlab does; packs/vue/NOTES-vue.md proposes it for the
engine), so `vscript` is empty. The engine instead builds the component's view (decision 0222,
`lib/vue/view.rerail`): one guest program of the scripts then the template, in the scripts' dialect. The rule now
includes `vue/view.rerail` and reads functions whose document is a `typescript`, `tsx` or `javascript` guest, as
the JSON's `embedded: script-block` does.

Two engine behaviours the rule follows:
- The `guest(g, _, _, dialect)` atoms must stand in the body that reads the library relations: the engine
  specialises `cc_report`'s kinds to the guest's grammar only there. Through a helper (`cc_vue_view(g)`) every
  kind constant resolved in the `vue` dialect and the rule reported nothing.
- The template's part of the view is code too: `options.vue:57`'s `v-for` becomes a function over the ceiling
  (its `:class` ternaries and the `v-if`/`v-else-if` code under it), which the rule reported before the exclusion.
  As the JSON's `tpl-block`, a function inside a `statement_block` whose `host_node` is the `template_element` is
  not scored. lizard reads only the scripts and reports nothing there either.

Known divergences from lizard, both the pack README's and kept by the original too: `switch` counts once
(`options.vue` `level`: lizard 7, the rule 2; `setup.vue` `switchOverLimit`: lizard 9, the rule 8).

**Library change proposed:** none needed. `vscript` and everything in `vue/component.rerail` stay empty in
the engine until it gives `parsed` rows for Vue scripts; rules needing a script's functions should read the view.
