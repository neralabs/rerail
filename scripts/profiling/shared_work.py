#!/usr/bin/env python3
"""Discover repeated SQL placements or check explicit shared producer declarations."""
import argparse
import hashlib
import json
from pathlib import Path

from shared_work.capture import discover
from shared_work.contracts import check


def read(path):
    def reject(value):
        raise ValueError(f"non-finite JSON value: {value}")
    return json.loads(path.read_text(), parse_constant=reject)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("discover", "check"):
        command = commands.add_parser(name)
        command.add_argument("inputs", nargs="+", type=Path)
        command.add_argument("--output", type=Path)
        if name == "discover":
            command.add_argument("--limit", type=int, default=20)
    args = parser.parse_args(argv)
    if args.output and args.output.resolve() in {p.resolve() for p in args.inputs}:
        parser.error("output cannot overwrite an input")
    try:
        documents = [(str(p), read(p)) for p in args.inputs]
        if args.command == "check":
            report = check(documents)
            success = report["valid"]
        else:
            if args.limit < 0:
                parser.error("limit must be nonnegative")
            report = {"format": "rerail.shared-work-discoveries/v1",
                      "reports": [discover(document, source, args.limit) for source, document in documents]}
            success = all(r["complete"] for r in report["reports"])
        report["inputs"] = [{"path": str(p), "sha256": hashlib.sha256(p.read_bytes()).hexdigest()}
                            for p in args.inputs]
        output = json.dumps(report, indent=2, allow_nan=False) + "\n"
        if args.output:
            args.output.write_text(output)
        else:
            print(output, end="")
        return 0 if success else 1
    except (ValueError, TypeError, KeyError, OSError) as error:
        parser.exit(2, f"shared-work: {error}\n")


if __name__ == "__main__":
    raise SystemExit(main())
