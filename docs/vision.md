# Vision and design principles

Where Rerail is going beyond code rules: the coordination problem it addresses, the semantic monitor at its core, and the principles behind it. Much of this is direction, not current behavior; the [README](../README.md) describes what ships today.

## The problem

AI increases the amount of engineering work a team can run in parallel. It also increases coordination failures:

- An agent crosses an architectural seam documented in a decision it never saw.
- Two engineers independently solve the same problem in incompatible ways.
- A known failed approach is repeated in another session.
- A requirement disappears between planning, implementation, and integration.
- Repository guidance arrives during review instead of while the change is being made.

Putting more text in every prompt does not solve this reliably. It consumes context, becomes stale, and still asks the generative model to recognize when a particular constraint applies.

Rerail moves that responsibility out of the model loop.

```text
Human input ──────────┐
Model output ─────────┼──▶ normalized events ──▶ local semantic monitor
Tool input and output ┘                              │
                                                    ▼
                                      relevant rule + match evidence
                                                    │
                                                    ▼
                                         log, steer, or block
```

The semantic layer answers:

> Which shared context is relevant to this activity?

It does not answer:

> Is this work correct, authorized, or safe?

Hard policy must remain deterministic. Semantic proximity is an attention signal, not proof of compliance or conflict.

## Vision

Rerail's destination is a live coordination layer for multiplayer engineering environments: every participant receives the relevant team decision, constraint, ownership signal, or prior evidence when their current work intersects it.

The central primitive is a stateful semantic monitor over team events:

```text
current human, agent, or tool event
                    ×
active team assertions and ongoing work
                    ↓
evidence-backed coordination finding
```

A team assertion might describe:

- An architectural invariant.
- A technical decision.
- Active work and its owner.
- A product requirement.
- A known failed approach.
- A required integration contract.

This enables several coordination workflows.

### Just-in-time engineering constraints

When work approaches authentication, migrations, public interfaces, persisted events, or another sensitive concept, Rerail surfaces the applicable repository guidance before the implementation diverges.

### Parallel-work overlap

Compare live activity with active assignments and recent work to identify likely duplication or incompatible approaches early. A semantic match should prompt coordination; it should never independently block or reassign work.

### Architectural alignment

Detect when an event is relevant to an architectural decision, then combine that relevance signal with deterministic evidence such as imports, paths, dependency direction, or tool targets.

### Engineer and agent handoffs

Preserve requirements, decisions, failed attempts, evidence, and open risks so the next participant receives the context relevant to the work they continue.

### Integration readiness

Relate parallel implementation results to shared requirements and integration contracts, highlighting weak coverage for human review without claiming that similarity proves correctness.

## What exists today

Rerail currently ships the local inspection kernel and host adapters:

- Native Model2Vec embeddings and cosine matching ([internal testing](../README.md#internal-testing)).
- Resident compiled rule references.
- User-input, streamed assistant-text, and tool-event inspection.
- Sentence, message, turn, and tool-call boundaries.
- Stateful repeat policies with session replay.
- One rule mode on every rule: `log` records, `steer` delivers guidance, and `block` steers until block enforcement lands (`block` is in [internal testing](../README.md#internal-testing)).
- OMP and Pi extensions.
- Transport-neutral Claude Code, Codex, and Cursor lifecycle adapters.
- A persistent `RERAIL-EVENTS/1` stream for out-of-process harnesses ([internal testing](../README.md#internal-testing)).
- Generic `mark`, `select`, `project`, and `relate` primitives for custom semantic workflows ([internal testing](../README.md#internal-testing)).
- A bundled, pinned INT8 model that makes no network requests at runtime.

Repository-level semantic rules work today, in internal testing. Shared team assertions, cross-session synchronization, ownership routing, acknowledgements, and coordination actions remain product direction rather than current behavior.

## Design principles

### Outside the generative model

Inspection must not depend on an agent deciding to invoke Rerail. Host adapters capture supported events automatically.

### Local by default

The model, vectors, and compiled references remain local. Runtime matching makes no network requests. The one thing Rerail sends about your work is a steer feedback note you approved, and only its rule, pack, coding agent, reason and Rerail version.

### Evidence before action

A match must retain its rule, source, boundary, score, threshold, turn, and model fingerprint. Downstream behavior should be auditable and replayable.

### Semantics for attention, determinism for enforcement

Cosine similarity can establish relevance. It cannot reliably prove contradiction, compliance, authorization, ownership, or correctness.

### Shared context without prompt saturation

Store team knowledge outside the prompt and deliver only the context relevant to the current event.

### Human authority

Potential overlap is a coordination signal. Humans and deterministic team policies decide whether to notify, acknowledge, redirect, or block work.
