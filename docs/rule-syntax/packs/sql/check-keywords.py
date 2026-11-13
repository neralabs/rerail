"""Check the RF04 and RF06 keyword tables against their policy JSON by a round trip: rebuild each JSON's list regex
from the table's rows, in table order, and look for it, byte for byte, among the JSON's regexes. It does not use
gen-keywords.py. Run: python3 check-keywords.py   (exit status 1 on any difference)
"""
import json, os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
RULES = os.path.normpath(os.path.join(HERE, "..", "..", "..", "..", "packs", "sql", "rules"))
CASES = {"sqlfluff-rf04-references-keywords": ("sql_keyword", "(?i)^(?:{})$"),
         "sqlfluff-rf06-references-quoting": ("pg_reserved", '(?i)^"(?:{})"$')}


def main():
    bad = 0
    for rule, (rel, shape) in CASES.items():
        ws = re.findall(rel + r'\("([^"]*)"\)\.', open(os.path.join(HERE, rule + ".facts.rr")).read())
        built = shape.format("|".join(ws))
        if json.dumps(built) in open(os.path.join(RULES, rule + ".json")).read():
            print(f"ok: {rule}: {len(ws)} words round-trip to the JSON's regex")
        else:
            bad += 1
            print(f"DIFFERENT: {rule}: the rebuilt regex is not in the JSON")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
