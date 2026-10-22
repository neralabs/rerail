{(a, b): 1 for ((a, b)) in x}
{((a, b),): 1 for (a, b), in x}
{(a, b): 1 for (a, b), in x}
{[a]: 1 for (a,) in x}
{a: 1 for ((a)) in x}
c: "{a: 1 for (a) in b}" = 1
d: "{(a, b): None for ((a, b)) in c}" = 1
e: "{a: a for a in b}" = 1
