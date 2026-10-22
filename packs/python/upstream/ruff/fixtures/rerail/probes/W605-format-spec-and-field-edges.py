a = b = 1
nested = f"{a:{b:\w}\d}"
field = f"\{a}\\{a}\\\{a}"
spec_end = f"{a!r:\d\\}"
spec_odd = f"{a:\\\}"
spec_even = f"{a:\\}"
spec_brace = f"{a:\{b}}"
spec_string = f"{a:{'x'}\%}"
bytes_n = b"\N\u\U\q"
cont = "x\
\q"
