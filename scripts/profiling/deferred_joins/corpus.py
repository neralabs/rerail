"""Prepare a pinned PostHog sample and capture one SLF001 producer with the native relational lab."""

import argparse
import hashlib
from functools import partial
import json
import os
from pathlib import Path
import signal
import sqlite3
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[3]
REV = "37c895112cd9830ba6161037c34c431a2a6d7d0c"
HEAD = "slf_class_head__sup2"
SAMPLES = {
    1644: (
        "2088b396c60754a3fa1d2cedf28f4502c959f41e836d9cde2309f1ae2653cdd4",
        "f45dc45b999b6a163f3dfcc3994d4d0b0a687239bd74e40757041760eb994553",
    ),
    1664: (
        "6280c34da046a08d6ab16e0fdb0fd0fb4201e311999796dbe3fd6ff5e627cf81",
        "34aa0a5f4014c9f835d95ddbd73bd1ac23a62a3feefebacf105404d1e87440f5",
    ),
}


def prepare(git, count, dest):
    tree = subprocess.check_output(
        ["git", "-C", git, "ls-tree", "-rz", REV, "--", "posthog"]
    )
    blobs = []
    for item in tree.split(b"\0"):
        if item.endswith((b".py", b".pyi")):
            meta, name = item.split(b"\t", 1)
            blobs.append((hashlib.sha256(name).digest(), name, meta.split()[2]))
    if len(blobs) < count:
        raise ValueError("the pinned tree contains too few Python files")
    digest = hashlib.sha256()
    dest.mkdir()
    with subprocess.Popen(
        ["git", "-C", git, "cat-file", "--batch"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
    ) as reader:
        for _, name, oid in sorted(blobs)[:count]:
            reader.stdin.write(oid + b"\n")
            reader.stdin.flush()
            found, kind, size = reader.stdout.readline().split()
            if found != oid or kind != b"blob":
                raise ValueError(f"missing blob for {name!r}")
            content = reader.stdout.read(int(size))
            if len(content) != int(size) or reader.stdout.read(1) != b"\n":
                raise ValueError("short Git blob")
            path = dest / os.fsdecode(name)
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)
            digest.update(name + b"\0" + hashlib.sha256(content).digest())
        reader.stdin.close()
        if reader.wait():
            raise RuntimeError("git cat-file failed")
    return digest.hexdigest()


class RunFailed(RuntimeError):
    def __init__(self, reason, metrics):
        super().__init__(reason)
        self.metrics = metrics


def bounded_run(command, out, env, rss_limit_mib, deadline_s, min_free_bytes=0, sample=False, accepted_codes=(0,)):
    start = time.monotonic()
    load = os.getloadavg()
    with (
        (out / "report.json").open("wb") as report,
        (out / "stderr.log").open("wb") as log,
    ):
        p = subprocess.Popen(
            ["nice", "-n", "19", *command],
            cwd=ROOT,
            stdout=report,
            stderr=log,
            env=env,
            start_new_session=True,
        )
        sampler = None
        stopped = None
        try:
            if sample:
                sampler = subprocess.Popen(
                    [
                        "nice",
                        "-n",
                        "19",
                        "/usr/bin/sample",
                        str(p.pid),
                        "120",
                        "10",
                        "-mayDie",
                        "-file",
                        out / "stacks.txt",
                    ],
                    stdout=log,
                    stderr=log,
                )
            while True:
                pid, status, usage = os.wait4(p.pid, os.WNOHANG)
                if pid:
                    p.returncode = os.waitstatus_to_exitcode(status)
                    elapsed = time.monotonic() - start
                    break
                rss = subprocess.run(
                    ["ps", "-p", str(p.pid), "-o", "rss="],
                    capture_output=True,
                    text=True,
                ).stdout.strip()
                if rss and int(rss) > rss_limit_mib * 1024:
                    stopped = f"{command[1]} exceeded {rss_limit_mib:,} MiB RSS"
                elif time.monotonic() - start > deadline_s:
                    stopped = f"{command[1]} exceeded {deadline_s} seconds"
                elif min_free_bytes:
                    fs = os.statvfs(out)
                    if fs.f_bavail * fs.f_frsize < min_free_bytes:
                        stopped = f"{command[1]} reached the free disk limit"
                if stopped:
                    try:
                        os.killpg(p.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    _, status, usage = os.wait4(p.pid, 0)
                    p.returncode = os.waitstatus_to_exitcode(status)
                    elapsed = time.monotonic() - start
                    break
                time.sleep(0.25)
        finally:
            if p.poll() is None:
                try:
                    os.killpg(p.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                except PermissionError:
                    p.kill()
                p.wait()
            if sampler is not None:
                try:
                    sampler.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    sampler.kill()
                    sampler.wait()
                if not p.returncode and (
                    sampler.returncode or not (out / "stacks.txt").is_file()
                ):
                    raise RuntimeError(f"sampling failed, see {out / 'stderr.log'}")
    peak_mib = usage.ru_maxrss / (1048576 if sys.platform == "darwin" else 1024)
    metrics = dict(
        elapsed_s=round(elapsed, 3),
        cpu_s=round(usage.ru_utime + usage.ru_stime, 3),
        load=load,
        peak_rss_mib=round(peak_mib, 2),
    )
    if stopped:
        raise RunFailed(stopped, metrics)
    if p.returncode not in accepted_codes:
        with (out / "stderr.log").open("rb") as stderr:
            stderr.seek(max(0, os.fstat(stderr.fileno()).st_size - 1500))
            reason = stderr.read().decode(errors="replace").strip()
        raise RunFailed(f"{command[1]} exited {p.returncode}: {reason}", metrics)
    if peak_mib > rss_limit_mib:
        raise RunFailed(f"native peak RSS reached {peak_mib:.1f} MiB (limit {rss_limit_mib:,} MiB)", metrics)
    return metrics


def native(
    engine,
    corpus,
    out,
    store,
    capture=None,
    *,
    mode=None,
    cache_rows=None,
    plan=False,
    policy=None,
    profile=False,
    sample=False,
    strict_pack=False,
    reach_index=True,
    library=None,
    rss_limit_mib=2250,
    deadline_s=120,
    min_free_bytes=0,
):
    out.mkdir()
    if policy is None:
        policy = (
            ROOT / "docs/rule-syntax/packs/python/ruff-slf001-private-member-access.rr"
        )
    env = {k: v for k, v in os.environ.items() if not k.startswith("RERAIL_")}
    env.update(
        RERAIL_THREADS="4",
        RERAIL_TAPE_CACHE=str(store),
        RERAIL_RR_LIB=str(library or ROOT / "docs/rule-syntax/lib"),
        TMPDIR=str(out),
    )
    if not reach_index:
        env["RERAIL_REACH_INDEX"] = "off"
    if strict_pack:
        env["RERAIL_BATCH_PACK"] = "strict"
    if mode:
        env["RERAIL_DEFERRED_JOINS"] = mode
    if cache_rows is not None:
        env["RERAIL_JOIN_CACHE_ROWS"] = str(cache_rows)
    if plan:
        env["RERAIL_DUMP_CROSS"] = str(out / "cross.rr")
    if profile:
        env["RERAIL_PROFILE"] = str(out / "profile")
    if capture:
        env.update(RERAIL_RELATIONAL_LAB=str(capture), RERAIL_RELATIONAL_LAB_ONLY=HEAD)
    policies = policy if isinstance(policy, (list, tuple)) else [policy]
    command = [engine, "run-policies", corpus, out / "output", *policies]
    metrics = bounded_run(command, out, env, rss_limit_mib, deadline_s, min_free_bytes, sample)
    report = json.loads((out / "report.json").read_text())
    if isinstance(policy, (list, tuple)):
        reported = sorted(run["path"] for run in report["runs"])
        if reported != sorted(str(path) for path in policy):
            raise RuntimeError(f"{mode} did not report every requested policy exactly once")
    rows = report["diagnostics"]
    canonical = sorted(
        json.dumps(r, sort_keys=True, separators=(",", ":")) for r in rows
    )
    return dict(
        diagnostics=len(rows),
        sha256=hashlib.sha256("\n".join(canonical).encode()).hexdigest(),
        **metrics,
    )


def adapt(path):
    """Match the support rule by its terms. A changed rule or extra literal needs a new adapter."""
    with sqlite3.connect(path) as db:
        (spec,) = db.execute("SELECT value FROM meta WHERE key='spec'").fetchone()
        lines = spec.splitlines()
        if len(lines) != 3 or lines[:2] != [f"name prog_{HEAD}", "rel r0 iiiiii empty"]:
            raise ValueError("unexpected capture component")
        prefix, index, encoded = lines[2].split()
        if (prefix, index) != ("seed", "0"):
            raise ValueError("expected one seed")
        body = bytes.fromhex(encoded).decode().splitlines()
        head = body.pop().split()
        if len(head) != 7 or head[0] != "head" or len(set(head[1:])) != 6:
            raise ValueError("unexpected support head")
        h, e, obj, attr, x, c = head[1:]
        limit = "i:6917529027641081856"
        tests = sorted(line.split() for line in body if not line.startswith("pos "))
        if tests != sorted([["lt", slot, limit] for slot in (h, e, c)]):
            raise ValueError("unexpected filters in captured rule")
        patterns = {
            (h, e): "asked",
            (h, "i:1025"): "kind",
            (e, c): "scope",
            (c, x): "name",
            (h, x): "text",
            (e, obj, attr): "attr",
        }
        sources = {}
        for line in body:
            if line.startswith("pos "):
                _, source, *terms = line.split()
                role = patterns[tuple(terms)]
                table = bytes.fromhex(source.split(":")[2]).decode()
                if (
                    not (table.startswith("s") and table[1:].isdecimal())
                    or role in sources
                ):
                    raise ValueError("unexpected capture source")
                sources[role] = table
        if set(sources) != set(patterns.values()):
            raise ValueError("incomplete source list")
        # These indexes belong to the offline SQLite capture. All four experiments share them.
        for role, table in sources.items():
            db.execute(f"CREATE INDEX ix_{role} ON {table}(c0,c1)")
            if role in ("text", "name", "scope"):
                db.execute(f"CREATE INDEX rev_{role} ON {table}(c1,c0)")
        bound = limit.split(":")[1]
        db.execute(f"""CREATE VIEW l(a,k) AS SELECT t.c0,t.c1 FROM {sources["text"]} t
                     JOIN {sources["kind"]} n ON n.c0=t.c0 AND n.c1=1025 WHERE t.c0<{bound}""")
        db.execute(f"""CREATE VIEW r(b,k) AS SELECT s.c0,n.c1 FROM {sources["scope"]} s
                     JOIN {sources["name"]} n ON s.c1=n.c0 WHERE s.c0<{bound} AND s.c1<{bound}
                     AND EXISTS (SELECT 1 FROM {sources["attr"]} t WHERE t.c0=s.c0)""")
        db.execute(
            f"CREATE VIEW requests(a,b) AS SELECT DISTINCT c0,c1 FROM {sources['asked']}"
        )
        db.execute("CREATE VIEW expected(a,b) AS SELECT DISTINCT c0,c1 FROM out0")
        db.execute("ANALYZE")
        return {
            name: db.execute(f"SELECT count(*) FROM {name}").fetchone()[0]
            for name in ("requests", "expected")
        }


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument(
        "--git",
        type=Path,
        required=True,
        help="PostHog clone containing the pinned revision",
    )
    p.add_argument("--engine", type=Path, required=True)
    p.add_argument("--count", type=int, choices=[1644, 1664], required=True)
    p.add_argument("--out", type=Path, required=True, help="new directory")
    p.add_argument(
        "--verify-only",
        action="store_true",
        help="compare a candidate engine cold and warm to the reference diagnostics without capturing its plan",
    )
    p.add_argument(
        "--check-edit",
        action="store_true",
        help="add one source in the temporary sample, compare with a cold run, then remove it",
    )
    p.add_argument(
        "--deferred",
        choices=["off", "on", "adaptive", "auto"],
        default="off",
        help="native planner mode to verify",
    )
    p.add_argument(
        "--check-plan",
        action="store_true",
        help="save the cold cross-file plan and check the class-head producer",
    )
    p.add_argument(
        "--profile", action="store_true", help="record native counters for the cold run"
    )
    p.add_argument(
        "--sample", action="store_true", help="sample the cold run's stacks on macOS"
    )
    p.add_argument("--cache-rows", type=int, help="pair witness allowance for this program")
    args = p.parse_args()
    if args.cache_rows is not None and args.cache_rows < 0:
        p.error("--cache-rows must be nonnegative")
    run = partial(native, mode=args.deferred, cache_rows=args.cache_rows)
    if args.sample and sys.platform != "darwin":
        p.error("--sample requires macOS")
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    engine = args.engine.resolve(strict=True)
    with tempfile.TemporaryDirectory(dir=out) as scratch:
        work = Path(scratch)
        corpus = work / "corpus"
        digest = prepare(args.git, args.count, corpus)
        if digest != SAMPLES[args.count][0]:
            raise ValueError("sample content differs from the pinned input")
        record = dict(
            revision=REV,
            files=args.count,
            deferred=args.deferred,
            cache_rows=args.cache_rows,
            corpus_sha256=digest,
            engine_sha256=hash_file(engine),
        )
        record["baseline"] = run(
            engine,
            corpus,
            out / "baseline",
            work / "baseline.sqlite",
            plan=args.check_plan,
            profile=args.profile,
            sample=args.sample,
        )
        if args.check_plan:
            rules = (out / "baseline/cross.rr").read_text().splitlines()
            head = [r for r in rules if r.startswith("slf_class_head(")]
            if not head or not all(
                "rr_product_demand_slf_class_head(" in r for r in head
            ):
                raise ValueError("the class-head producer lost its correlated request")
            uses_factor = any("rr_deferred_" in r and "_factor(" in r for r in head)
            if args.deferred in ("adaptive", "auto") and not uses_factor:
                raise ValueError(
                    "the class-head producer did not select fanout sharing"
                )
            record["class_head_factor"] = uses_factor
        if not args.verify_only:
            record["capture"] = run(
                engine,
                corpus,
                out / "capture-run",
                work / "capture.sqlite",
                work / "capture",
            )
        warm_store = work / (
            "baseline.sqlite" if args.verify_only else "capture.sqlite"
        )
        record["warm"] = run(
            engine, corpus, out / "warm", warm_store
        )
        for label in ("baseline", "warm", "capture"):
            if label in record and record[label]["sha256"] != SAMPLES[args.count][1]:
                raise ValueError(f"{label} diagnostics differ from the reference")
        if args.check_edit:
            extra = corpus / "posthog/_deferred_join_lab_probe.py"
            with extra.open("x") as source:
                source.write("def probe(other):\n    return other._join_lab_probe\n")
            record["insert"] = run(
                engine, corpus, out / "insert", warm_store
            )
            record["insert_control"] = run(
                engine,
                corpus,
                out / "insert-control",
                work / "edit-control.sqlite",
            )
            rows = json.loads((out / "insert/report.json").read_text())["diagnostics"]
            if not any(
                r["path"].endswith(extra.name)
                and r["parameters"].get("attr") == "_join_lab_probe"
                for r in rows
            ):
                raise ValueError("the edit did not produce its witness diagnostic")
            if record["insert"]["sha256"] != record["insert_control"]["sha256"]:
                raise ValueError(
                    "the updated store disagrees with the cold edit control"
                )
            extra.unlink()
            record["delete"] = run(
                engine, corpus, out / "delete", warm_store
            )
            if record["delete"]["sha256"] != SAMPLES[args.count][1]:
                raise ValueError(
                    "removing the probe did not restore the reference diagnostics"
                )
        if not args.verify_only:
            files = list((work / "capture").glob(f"prog_{HEAD}-*.sqlite"))
            if len(files) != 1:
                raise ValueError(f"expected one captured stratum, got {len(files)}")
            record.update(adapt(files[0]))
            files[0].rename(out / "case.sqlite")
    (out / "manifest.json").write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps(record), flush=True)


def hash_file(path):
    with path.open("rb") as stream:
        h = hashlib.sha256()
        for block in iter(lambda: stream.read(1 << 20), b""):
            h.update(block)
        return h.hexdigest()


if __name__ == "__main__":
    os.nice(19)
    main()
