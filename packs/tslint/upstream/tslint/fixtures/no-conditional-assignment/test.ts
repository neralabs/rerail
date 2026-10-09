// valid cases
if (x == 5) { }
if (x === 5) { }
else if (y <= 23) { }
else if ((z && a) == 7) { }
else { }

do { } while (x == 2);
do { } while (x !== 2);

while (x == 2) { }
while (x !== 2) { }

for (var x = 8; x == 8; ++x) { }
for (var x = 8; x == 8; x = 12) { }
for (;;) { }

while (a ? b : c) {}
while (--i) {}
while ((a = b)++) {} // don't bother, it's a compile error anyway

while ((a || (a = b)).foo) {}
while (a[b = c]) {}
while ((() => a = b)()) {}
while (fn(a = b)) {}
a ? b = a : a = b;

// invalid cases
if (x = 5) { }
if (a && (b = 5)) { }
else if (x = 2) { }

do { } while (x = 4);

while (x = 4);
while ((x = y - 12));

for (var x = 4; x = 8; x++) { }
for (; (y == 2) && (x = 3); ) { }

if (x += 2) { }
else if (h || (x <<= 4)) { }

do { } while (x ^= 4) { }
while ((a = 5) && ((b == 4) || (c = 3))) {}
while (+(a = b) || -(b = c)) {}
while (foo() || (a = (b = c)!)) {}
while (a == (b = fn()) as any) {}
while (a ? b = a : a = b) {}
while (!((a = b) ? c : d) || (x = y)) {}
(a = b) ? c : d;
(foo ? a = b : b = a) ? c : d;

