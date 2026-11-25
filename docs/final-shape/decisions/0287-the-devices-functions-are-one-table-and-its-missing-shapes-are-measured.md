# 0287: The device's functions are one table, and its missing shapes are measured first

Numbered 0286 where it was written: django-bugs has its own 0286.

Status: partly built (audited 2026-11-23; §2 built 2026-11-24). Left: §3–§4 drafts: made texts
inside recursion, remaining functions (deferred until a workload shows gain). §1 built (2026-11-15), §2 built (2026-11-24); §3–§4 draft, not
to be built before a workload shows their gain. Refines the device's placement (`plan::placement`, decision 0095) and
its compile (`setfix::device`).

## Intro

The device has only some of the forms a stratum's terms can take. When a stratum needs one it lacks, placement keeps
the stratum on the CPU, and the profile names the term ("computed term: @rr_node_text in a comparison"). Many text
functions were added to the device recently. This record asks which shapes are still missing, which of them cost
anything, and how to build the ones that do.

Two kinds of lists said what the device can compute:

- **placement** (in `rerail-logic`): `DEVICE_TEXT_FUNCTIONS`, `DEVICE_ARITHMETIC`, and the names `str_cmp` and
  `strlen`, written as the planner's terms write them (`substr`, `add`), with some native names mixed in;
- **the device's compile** (in `rerail-device-metal`): `text_function` and `arith_op`, by the native engine's names
  (`rr_substr`, `rr_add`).

These were kept in step by hand. A function added on one side and not the other was either placed and then refused at
open, or never placed at all.

## Measured: what is missing, and what it costs

These are two cold runs with Metal on, on an M-series Mac, after the hub's merge (3eb66abed).

**The light run** is one rule (`builtins.len` calls) over PostHog's `posthog/hogql`, 437 files:

- 181 strata, 1,630 findings, the same findings as on the CPU;
- 8 strata ran on the GPU;
- 137 strata stayed on the CPU because the price said so (1.67 s CPU);
- the strata a missing shape kept off the GPU hold about 0.2–0.5 s.

**The pack run** is the Python pack's 544 rules over 20 of those files, 9.5 s CPU. That run's binding was the CPU, so
the profile names each stratum's missing shapes without running any of them on the GPU. The strata that lack a shape
hold about 0.1 s in all.

| Missing shape | Strata (light / pack) |
|---|---|
| `@rr_node_text` in a comparison (a node's text past the 256-byte inline bound) | 15 / 125 |
| `@rx_group` | 4 / 24 |
| `@cat` / `@concat` in a head or a comparison (nested, many parts, or in a recursive stratum) | 8 / 32 |
| `@str_between` | 1 / 14 |
| string join | – / 11 |
| `@to_number`, `@to_float` | – / 15 |
| `@pkg_up`, `@json_str`, `@rr_node_text_any`, `@rr_node_value` | a few |
| `@head`, `@substr`, `@slice`, `@replace_all` inside a recursive stratum | a few |
| `split` (refused by the device's compile; seen in the django session's runs) | – |

What decides the order of work:

1. **The price would keep most of them on the CPU anyway.** The profile prices every stratum on both devices and lists
   the costliest ones the GPU would run cheaper.
   - In the light run that list holds no stratum blocked by `@rr_node_text`. Only `qualname` (a recursive `@cat`) was
     on it, at 0.086 s.
   - In the pack run, one of the top 20 is blocked by a shape: `p524_attempt`, 0.048 s.
2. **The long-text twin is not empty.** Each rule that reads `text(n, x)` gets a twin, `rr_long_text_node(n),
   x = @rr_node_text(n)`. `rr_long_text_node` holds every node whose text is past 256 bytes, so every large function
   body is in it. The twin makes no rows because its other atoms match no long node, not because its input is empty.
   - So leaving out seeds over an empty input misses this case. That fix was built and measured: in the light run the
     same 15 strata were still refused, and it was not kept.

## Decision

### §1 One table of the device's functions (built)

`logic::device::caps` is the single table: for each native function name, its device form.

- **`DeviceFn::Arith`**: integer arithmetic of two numbers.
- **`DeviceFn::Names`**: `str_cmp` and `rr_strlen`, read where the names already are.
- **`DeviceFn::Text`**: a made text, with how many text arguments and how many numbers it takes.
- **`DeviceFn::RegexReplace`**: `replace_all` and `rx_replace`.

The table never names an operation code, so it stays in `rerail-logic` (which names no device), where placement can
read it. Each device maps the entries to its own codes. Metal does so in `setfix::device`'s `text_function` and
`arith_op`, as exhaustive matches, so an entry without a Metal operation does not compile. `caps::native` turns the
planner's `.rr` names into the native ones (`strings::functions::native_call`). Placement now checks a function's
argument count against the table for every form; before, only arithmetic was checked.

Test: `caps::tests::the_planners_names_and_the_devices_are_one_table`. Each `.rr` name reaches its form, and each
form takes as many arguments as the native engine's function.

### §2 Node texts read from the source on the device (built 2026-11-24)

Built: `@rr_node_text_any` and `@rr_node_text` are text functions of the device (`caps::TextFn::NodeTextAny`,
`NodeText`; `TEXTFN_NODE_TEXT_ANY`, `TEXTFN_NODE_TEXT`). No upload: the views' directory names each document's own
source as a home of its row (`ViewRow::source`, `homes[2]`), and the stage reads a node's bytes there. The node's
document is found by its key, then its numbered node, then its range in its own source (`view_text_span`, written
once in CLIF for the JIT, the kernels and the Rust reads). The bytes are made into the arena by `row_text_one`, as
any text function's. The stage's probed relation is the batch's `node` view (`Source::node_view`, from the run's
binding of the long-text nodes, `LongTexts`), which binds the directory and makes the sources' slabs resident. A
batch whose sources the device does not read (`ViewsInPlace::sources`), and a node the directory does not hold (a
comment span, another batch's), run on the host. Tests: `a_nodes_text_on_the_device_is_read_where_its_source_lies`
(the CPU's rows are the host's `node_read`, the device's rows and counts per step the CPU's, on CUDA),
`the_view_reads_are_the_hosts_rows` (the span and the text, every node, both formats).

A line's text too (2026-11-24, 0353 row 7). `@rr_node_text_any` of a document's `source` span or a `src_line` span
(read so on the host since b6c6e26f34) is read where its source lies on the device, from three reads written once in
CLIF beside the node's:
- `view_line_span(low)` decodes the span's id (`rerail_clif::ids::LINE_SLOT`, the facts' `ExtraSpan::Line` slot,
  asserted one) and gives the line's bytes from its first to one past its terminator's last, by the lines' index;
- `view_line_terminated(low)` says whether the line ends with a terminator;
- `lines_terminator_length(last)`, a read of the lines' family whose base is the source's home (`homes[2]`), reads
  the terminator's length off the source's bytes, as `newlines::length_ending_at`.

A guest's lines are its host's, so its line spans are read on the host (`NO_SPAN`), and so is any line where a
document has no core.rerail positions (`own_text` 0). `@rr_node_text` (the long-text twin) has no row for a line, as
the host's `long_text`. The Rust twins are `Doc::line_span`, `line_terminated` and `line_text`. Tests:
`the_view_reads_are_the_hosts_rows` (every line's span and terminator, the JIT's and the Rust reads', and its text
`LineTable::text_of`'s, with CRLF, a lone CR, U+2028 and no final terminator among its cases), and
`a_nodes_text_on_the_device_is_read_where_its_source_lies` (each document's line spans among the nodes asked, in
core.rerail's shape: the device's rows and counts per step are the CPU's, on CUDA).

The draft as written:

- `@rr_node_text(n)` is the node's byte range in its document's source: `node(d, n, _, start, end)`.
- The device's compile would rewrite the step as a keyed read of `node` (binding `start` and `end`), followed by a
  `ROWTEXT` stage that copies `source[d][start..end)` into the text arena.
- From there every existing reader of arena texts takes it: `=` and `!=`, text tests, text functions, and the head's
  make.
- This needs the batch's sources uploaded, once per batch and only for strata that read them.
- Comment spans and `@rr_node_value` (a literal's decoded value) stay on the CPU, named.
- Cost: a new kernel stage, an upload path, and parity tests. Measured gain on both runs above: none.

### §3 Made texts inside recursion (draft)

- In a recursive stratum such as `qualname`, a text made in one round is joined in the next, so it needs an id before
  the round ends.
- The device would keep a table of the round's new texts, keyed by bytes, which hands out provisional ids, so that a
  made text equal to an existing name gets that name's id.
- After the fixpoint, the host turns the provisional ids into real atoms in one pass.
- This would also admit `head`, `substr`, `slice` and `replace_all` inside recursion. Measured gain: `qualname`,
  0.086 s in the light run.

### §4 The remaining functions (draft)

Each is a new entry in §1's table plus its kernel:

- `str_between`, `pkg_up` and `json_str` as text functions;
- `to_number` and `to_float` as text-to-number operations, like `strlen`;
- `rx_group` on the regex-replace automaton, keeping the group's range;
- concatenation of many parts, or nested, written straight into the arena;
- `split`, as a step that makes a row per piece;
- string join, as an aggregate: sort by group and order, then concatenate.

Most of these uses sit in the import-request (`ask_*`) components. Those handle a few rows a run, and the device
refuses components with requests anyway.

## Consequences

- A function added to the device is one table entry and one match arm. Placement admits it with no list to update.
- §2–§4 wait for a workload where the strata they unblock are ones the price would put on the GPU. The profile's
  list of the costliest strata the GPU would run cheaper is the test: a stratum belongs to this work only if it is
  on that list with a missing shape.
