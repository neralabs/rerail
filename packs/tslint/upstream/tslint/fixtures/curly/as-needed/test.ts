if (so) {
    foo();
} else {
    foo();
}

while (true) {
    foo();
}

if (so) {
    if (also)
        foo();
}

if (so) {
    if (also)
        foo();
    else
        foo();
} else
    foo();

if (so)
    bar();
else {
    if (also)
        foo();
}

// Some blocks are necessary.

if (so) {
    if (also)
        foo();
} else
    bar();

function f() {
    foo();
}

() => { foo(); };

try { foo(); } catch (e) { foo(); } finally { foo(); }

