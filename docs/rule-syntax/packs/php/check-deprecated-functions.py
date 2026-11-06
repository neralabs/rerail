"""Check the deprecated-function table against its policy JSON by a round trip: rebuild the JSON's
`resolves.import` list from the table's rows, in table order, and compare. It does not use
gen-deprecated-functions.py. Run: python3 check-deprecated-functions.py   (exit status 1 on any difference)
"""
import json, os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
RULE = "phpcs-generic-php-deprecated-functions"
JSON = os.path.normpath(os.path.join(HERE, "..", "..", "..", "..", "packs", "php", "rules", RULE + ".json"))


def main():
    rows = re.findall(r'deprecated_fn\("([^"]*)"\)\.', open(os.path.join(HERE, RULE + ".facts.rr")).read())
    want = json.load(open(JSON))["clauses"][0]["forbid"]["ast"]["rule"]["has"]["resolves"]["import"]
    if rows == want:
        print(f"ok: {RULE}: {len(rows)} functions round-trip to the JSON's list")
        return 0
    print(f"DIFFERENT: {RULE}: table {len(rows)} rows, JSON {len(want)} names")
    return 1


if __name__ == "__main__":
    sys.exit(main())
