# T's programs, pinned

T's `rdl/*.rdl` as copied (Typing-support@7ebcffe0, `091b65fb`), unchanged. The tests that reproduce T
read these: `logic::front` (parse and strata counts), `logic::magic` (`rdl.py`'s demand, adornment and
gate outputs) and `logic::plan` (T's chosen plans and outputs). Since the plans are pinned rule by rule,
they need T's text exactly.

The product's programs are `typing/programs/`. They are mutated from these: for example, no
hand-written demand (list step 29). `logic::plan`'s
`product_programs_derive_t_outputs` checks that they still derive T's outputs.
