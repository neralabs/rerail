"""Convert macOS sample's weighted call tree to bottom-up self/inclusive counts."""
import collections
import re

FRAME = re.compile(r"^([ +!|:]*)\s*(\d+) (.+)$")
SYMBOL = re.compile(r"^(.+?)\s+\(in ([^)]+)\)(?:.*)$")
WAIT = re.compile(r"^(?:__psynch_cvwait|__psynch_mutexwait|_pthread_cond_wait|kevent|semaphore_wait_trap|__workq_kernreturn|mach_msg2_trap)$")


def bottom_up(source, modules=("rerail_engine",)):
    """Count weighted residuals at every node, including truncated non-leaf stacks.

    Only stacks containing selected modules enter the active denominator.
    The wildcard selects all active stacks, including a standalone SQL host.
    Parked threads are separate; inclusive recursion counts once per sampled stack.
    """
    stack, leaves = [], []
    thread = "unknown"
    in_graph = False

    def finish():
        depth, count, symbol, children = stack.pop()
        residual = count - children
        if residual < 0:
            raise ValueError("sample child weights exceed parent")
        if residual:
            leaves.append((residual, [item[2] for item in stack] + [symbol], thread))

    for line in source.splitlines():
        if line == "Call graph:":
            in_graph = True
            continue
        if not in_graph:
            continue
        if line.startswith("Total number in stack") or line.startswith("Binary Images:"):
            break
        owner = re.match(r"^    \d+ (Thread_\d+[^\n]*)", line)
        if owner:
            while stack:
                finish()
            thread = owner[1]
            continue
        match = FRAME.match(line)
        if match is None:
            continue
        prefix, weight, frame = match.groups()
        symbol = SYMBOL.match(frame)
        name, module = symbol.groups() if symbol else (frame, "unknown")
        depth = len(prefix)
        while stack and stack[-1][0] >= depth:
            finish()
        count = int(weight)
        if stack:
            stack[-1][3] += count
        stack.append([depth, count, f"{name.strip()} ({module})", 0])
    while stack:
        finish()
    self_counts, inclusive, callers = collections.Counter(), collections.Counter(), collections.Counter()
    active = parked = other = 0
    waits = collections.Counter()
    threads = collections.Counter()
    subsystems = collections.Counter()
    for count, frames, owner in leaves:
        if WAIT.match(frames[-1].split(" (")[0]):
            parked += count
            waits[frames[-1]] += count
            continue
        if "*" not in modules and not any(module in frame for module in modules for frame in frames):
            other += count
            continue
        active += count
        threads[owner] += count
        # Assign each native sample once, using its full caller chain. Allocator
        # and comparison leaves belong to SQLite when SQLite called them.
        if any("code_rule_dsl" in frame and "profile" in frame for frame in frames):
            subsystem = "profiling"
        elif any("rerail_reachability_spike::graph" in frame for frame in frames):
            subsystem = "reachability-graph"
        elif any("rerail_reachability_spike::table" in frame for frame in frames):
            subsystem = "reachability-interface"
        elif any(re.search(r"\bsqlite3\w*", frame) for frame in frames):
            subsystem = "sqlite"
        elif any("ast_grep" in frame or "code_rule_dsl" in frame and any(part in frame for part in ["matcher", "tape", "compile"]) for frame in frames):
            subsystem = "matching-and-parsing"
        elif any("code_rule_dsl" in frame and "relational" in frame for frame in frames):
            subsystem = "relational-host"
        else:
            subsystem = "other-native"
        subsystems[subsystem] += count
        self_counts[frames[-1]] += count
        inclusive.update({frame: count for frame in set(frames)})
        callers[tuple(reversed(frames[-7:]))] += count
    rows = lambda counter: [{"symbol": key, "samples": count, "percent": round(100 * count / active, 2) if active else 0} for key, count in counter.most_common()]
    return {"sampleModules":list(modules), "activeNativeSamples": active, "parkedThreadSamples": parked, "otherActiveSamples": other,
            "waitSymbols": dict(waits),
            "self": rows(self_counts), "inclusive": rows(inclusive),
            "bottomUpStacks": [{"frames": list(key), "samples": count} for key, count in callers.most_common()],
            "nativeThreadSamples": dict(threads), "subsystems": rows(subsystems)}
