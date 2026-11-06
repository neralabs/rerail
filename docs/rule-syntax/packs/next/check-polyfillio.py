"""Check next-no-unwanted-polyfillio.facts.rr against the policy JSON by a round trip: rebuild the JSON's src regex
from the table's rows (hosts and features, in table order, escaped as the JSON escapes them) and compare it with the
shipped regex, byte for byte. Run: python3 check-polyfillio.py   (exit status 1 on any difference)
It does not use gen-polyfillio.py's parser.
"""
import json, os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
JSON = os.path.normpath(os.path.join(HERE, "..", "..", "..", "..", "packs", "next", "rules",
                                     "next-no-unwanted-polyfillio.json"))
FACTS = os.path.join(HERE, "next-no-unwanted-polyfillio.facts.rr")


def esc(s):
    return re.sub(r"([.?*+()\[\]{}|^$\\])", r"\\\1", s).replace("@", "(?:@|%40)")


def main():
    text = open(FACTS).read()
    hosts = re.findall(r'polyfill_host\("([^"]*)"\)\.', text)
    feats = re.findall(r'polyfilled\("([^"]*)"\)\.', text)
    built = ("^(?:" + "|".join(map(esc, hosts)) + r")[^?#]*\?(?:[^&#]*&)*features=(?:[^&#]*?(?:,|%2[Cc]))?(?:"
             + "|".join(map(esc, feats)) + r")(?:(?:,|%2[Cc])|&|#|$)")
    shipped = set(re.findall(r'"regex": "(\^\(\?:https[^"]*features=[^"]*)"', open(JSON).read()))
    shipped = {json.loads(f'"{s}"') for s in shipped}
    if shipped == {built}:
        print(f"ok: {len(hosts)} hosts, {len(feats)} features round-trip to the JSON's regex")
        return 0
    print("DIFFERENT\n built:   " + built + "\n shipped: " + "\n          ".join(sorted(shipped)))
    return 1


if __name__ == "__main__":
    sys.exit(main())
