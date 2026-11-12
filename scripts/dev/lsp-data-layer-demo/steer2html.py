"""Demo: the steer text an agent receives, as the same window frame as the editor screenshots.

Usage: steer2html.py <steer.txt> <window title>
"""
import html
import sys

text = open(sys.argv[1], encoding="utf-8").read().rstrip("\n")
title = sys.argv[2]
lines = []
for line in text.split("\n"):
    escaped = html.escape(line)
    if line.startswith("Rerail found"):
        escaped = f'<span style="color:#f5a97f;font-weight:bold">{escaped}</span>'
    elif line.startswith("- "):
        place, _, rest = escaped[2:].partition(" — ")
        message, _, rule = rest.rpartition(" [")
        escaped = f'- <span style="color:#8bd5ca">{place}</span> — {message} <span style="color:#a6adc8">[{rule}</span>'
    else:
        escaped = f'<span style="color:#a6adc8">{escaped}</span>'
    lines.append(escaped)
print(f"""<!doctype html><meta charset="utf-8"><title>{html.escape(title)}</title>
<style>
body{{margin:0;background:#1e1e1e;padding:24px;font-family:-apple-system,sans-serif}}
.window{{display:inline-block;border-radius:10px;overflow:hidden;box-shadow:0 12px 40px rgba(0,0,0,.6);border:1px solid #3a3a3a}}
.bar{{background:#2d2d2d;color:#bbb;font-size:13px;padding:8px 12px;display:flex;gap:8px;align-items:center}}
.dot{{width:12px;height:12px;border-radius:50%;display:inline-block}}
.body{{background:#14161b;padding:14px 18px}}
.label{{color:#7f849c;font:12px Menlo,monospace;margin-bottom:8px}}
pre{{margin:0;padding:10px 14px;border-left:3px solid #f5a97f;background:#1b1e25;color:#e0e2ea;font:13px/1.5 Menlo,'SF Mono',monospace;white-space:pre}}
</style>
<div class="window"><div class="bar"><span class="dot" style="background:#ff5f57"></span><span class="dot" style="background:#febc2e"></span><span class="dot" style="background:#28c840"></span><span style="margin-left:8px">{html.escape(title)}</span></div>
<div class="body"><div class="label">steer delivered to the agent after its edit of l6500_evidence/capability_store.py</div>
<pre>{chr(10).join(lines)}</pre></div></div>""")
