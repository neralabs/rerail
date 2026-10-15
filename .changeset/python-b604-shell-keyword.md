---
"@neralabs/rerail-pack-python": patch
---

bandit-b604 (candidate) reads only a `shell=` keyword, as Bandit's has_shell does: a call's second positional argument is no longer taken as `shell` (pytest.mark.parametrize, getattr and re.sub stop being flagged), and `shell=<expression>` is now reported unless it is a falsy constant, `[]` or `{}`.
