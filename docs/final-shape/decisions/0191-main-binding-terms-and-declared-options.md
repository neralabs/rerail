# 0191: Main's binding terms read through `bind_name` and `main_form`; a rule's declared options are its defaults

Status: decided (2026-11-13, in the work to run main's rules directly: "please take it", on the binding-site gap
decision 0182 recorded).

## Intro

Main's rules ask about a name's binding with `binds`:

```
// N806: an identifier bound in a function by an assignment, a walrus, a `for` …, not in the ignored names
{kind: identifier, binds: {form: [assignment, annotated, named, for, …], scope: function},
 not: {regex: {$option: pep8-naming.ignore-names, join: "|", template: "^(?:{})$"}}}
```

Three things kept this from meaning what main means.

- **The node.** Main's `binds` holds at the binding's name node (`Bad` in `Bad = 1`), and its filters are about the
  binding. The library's binding relations are keyed by the binding's site: the assignment statement, the `def`, the
  import item. The translator tested the name as if it were the site, so `Bad = 1` had no binding at all. Only a
  `for` or `with` target, whose site is the name itself, was found.
- **The forms.** Main says `assignment`, `annotated`, `annotation`, `augmented`, `named`, `definition`,
  `parameter`, `typeParameter`. The library says `assign`, `augassign`, `walrus`, `def`, `class`, `param`, and has
  no bare-annotation or type-parameter binding.
- **The options.** A list option's regex (`join` + `template`) was read with `option(name, o)` as if it were one
  string. Also, a rule's `.options` defaults never reached the run: the `logic` command passes no policy options.
  So the ignored names were never ignored.

Fixing the first two exposed a fourth problem. A `transform` whose source a rule binds only inside a negated util
(N806's `MODEL`) was computed in the main rule, which grounded the source over every node. That pairs every name
with every node, and the negation then held for some unrelated node, so names were reported that should not be.

## Decision

1. **`bind_name(n, site)` and `main_form(site, form)` are the interface for main's binding terms.**
   `lib/python/names.rr` and `lib/tsx/names.rr` each define them:
   - `bind_name` maps a name node to the library's site. That is the assigned or defined identifier, an unaliased
     import's `dotted_name`, or an import's alias.
   - `main_form` gives main's form names.

   The library's own `site_form` and its names stay as they are, for the hand-written rules.
2. **The translator writes `binds` as `bind_name(v, b)`, and puts the filters on `b`.** It writes forms as
   `main_form(b, f)`, and in `binding` and `redefines` too.
3. **Python gains the two bindings main has and the library lacked, as sites with a scope only.** A bare
   annotation `x: T` (`ann_site`) is Ruff's `Annotation`: it exists only where the scope binds the name nowhere
   earlier. A PEP 695 type parameter (`tp_site`) is the other. Neither is a `binding`, so reads never resolve to
   them. Main's `global` and `deletion` sites are still missing.
4. **A list option is read per item.** `{"$option": L, "join": "|", "template": T}` is
   `option_item(L, _, o), r = cat(pre, cat(o, post)), match(r, n)`, where `pre` and `post` are T around its `{}`.
   The joined regex matches exactly where the templated regex of one item does. A scalar option with a `template`
   is put in place the same way.
5. **The run's options are laid over the defaults the programs declare** (`repository::with_declared`). An option
   the caller does not set has its `.options` default, in every run.
6. **A transform joins a rule only where the rule binds its source outside negations and aggregates.** If not, the
   helper that reads the target computes it in its own frame, and a target the head names is `unset`.

## Consequences

- On a probe of 30 lines, N806 reports exactly the expected names. These include `Bad = 1`, the walrus, and a
  method's local. It skips `setUp` (ignored by default) and `User = apps.get_model("auth", "User")`. Its cost on
  the probe went from 8.2 s and 750 MB (with the grounded transform) to 0.7 s and 177 MB.
- 65 generated rules change. All 537 still pass the reader, and the Ruff fixture comparison is unchanged.
- Every rule with a list option of names (the N8xx `ignore-names`, and others) now honours its defaults.
