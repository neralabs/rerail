"""Demo: a tmux `capture-pane -e` screen (truecolor SGR) as an HTML page, for a browser screenshot.

Usage: ansi2html.py <screen.ansi> <default fg> <default bg> <window title>
"""
import html
import re
import sys

SGR = re.compile(r"\x1b\[([0-9;:]*)m")
BASE = ["#000000", "#cd3131", "#0dbc79", "#e5e510", "#2472c8", "#bc3fbc", "#11a8cd", "#e5e5e5"]
BRIGHT = ["#666666", "#f14c4c", "#23d18b", "#f5f543", "#3b8eea", "#d670d6", "#29b8db", "#ffffff"]
TARGET = {"38": "fg", "48": "bg", "58": "ul"}


def color256(n):
    if n < 8:
        return BASE[n]
    if n < 16:
        return BRIGHT[n - 8]
    if n < 232:
        n -= 16
        steps = [0, 95, 135, 175, 215, 255]
        return "#%02x%02x%02x" % (steps[n // 36], steps[(n // 6) % 6], steps[n % 6])
    level = 8 + (n - 232) * 10
    return "#%02x%02x%02x" % (level, level, level)


def extended(values):
    """The colour of `2;r;g;b` or `5;n`."""
    if values[0] == "2":
        return "#%02x%02x%02x" % tuple(int(x or 0) for x in values[1:4])
    return color256(int(values[1]))


def apply(state, params):
    tokens = params.split(";") if params else ["0"]
    i = 0
    while i < len(tokens):
        token = tokens[i]
        if ":" in token:  # colon subparameters: 4:3 (curly underline), 38:2::r:g:b, 58:2::r:g:b
            head, *sub = token.split(":")
            if head == "4":
                state["underline"] = sub[0] != "0"
                state["curly"] = sub[0] == "3"
            elif head in TARGET:
                state[TARGET[head]] = extended([s for s in sub if s != ""])
            i += 1
            continue
        n = int(token or 0)
        if str(n) in TARGET:
            width = 5 if tokens[i + 1] == "2" else 3
            state[TARGET[str(n)]] = extended(tokens[i + 1:i + width])
            i += width
            continue
        if n == 0:
            state.clear()
        elif n in (1, 3, 4, 7):
            state[{1: "bold", 3: "italic", 4: "underline", 7: "reverse"}[n]] = True
        elif n in (22, 23, 27):
            state.pop({22: "bold", 23: "italic", 27: "reverse"}[n], None)
        elif n == 24:
            state.pop("underline", None)
            state.pop("curly", None)
        elif 30 <= n <= 37:
            state["fg"] = BASE[n - 30]
        elif 90 <= n <= 97:
            state["fg"] = BRIGHT[n - 90]
        elif 40 <= n <= 47:
            state["bg"] = BASE[n - 40]
        elif 100 <= n <= 107:
            state["bg"] = BRIGHT[n - 100]
        elif n in (39, 49, 59):
            state.pop({39: "fg", 49: "bg", 59: "ul"}[n], None)
        i += 1


def style(state, default_fg, default_bg):
    fg, bg = state.get("fg", default_fg), state.get("bg", default_bg)
    if state.get("reverse"):
        fg, bg = bg, fg
    css = [f"color:{fg}", f"background:{bg}"]
    if state.get("bold"):
        css.append("font-weight:bold")
    if state.get("italic"):
        css.append("font-style:italic")
    if state.get("underline"):
        css.append(f"text-decoration:underline {'wavy' if state.get('curly') else 'solid'} {state.get('ul', fg)}")
    return ";".join(css)


def main():
    raw = open(sys.argv[1], encoding="utf-8").read().rstrip("\n")
    default_fg, default_bg, title = sys.argv[2], sys.argv[3], sys.argv[4]
    rows = []
    state = {}
    for line in raw.split("\n"):
        out = []
        pos = 0
        for match in SGR.finditer(line):
            if match.start() > pos:
                out.append(f'<span style="{style(state, default_fg, default_bg)}">{html.escape(line[pos:match.start()])}</span>')
            apply(state, match.group(1))
            pos = match.end()
        if pos < len(line):
            out.append(f'<span style="{style(state, default_fg, default_bg)}">{html.escape(line[pos:])}</span>')
        rows.append("".join(out))
    print(f"""<!doctype html><meta charset="utf-8"><title>{html.escape(title)}</title>
<style>
body{{margin:0;background:#1e1e1e;padding:24px;font-family:-apple-system,sans-serif}}
.window{{display:inline-block;border-radius:10px;overflow:hidden;box-shadow:0 12px 40px rgba(0,0,0,.6);border:1px solid #3a3a3a}}
.bar{{background:#2d2d2d;color:#bbb;font-size:13px;padding:8px 12px;display:flex;gap:8px;align-items:center}}
.dot{{width:12px;height:12px;border-radius:50%;display:inline-block}}
pre{{margin:0;padding:6px 8px;background:{default_bg};color:{default_fg};font:13px/1.35 Menlo,'SF Mono',monospace;white-space:pre}}
</style>
<div class="window"><div class="bar"><span class="dot" style="background:#ff5f57"></span><span class="dot" style="background:#febc2e"></span><span class="dot" style="background:#28c840"></span><span style="margin-left:8px">{html.escape(title)}</span></div>
<pre>{chr(10).join(rows)}</pre></div>""")


main()
