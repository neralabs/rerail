var f = 1, l = function(x) { return { u1: x } }; switch (f) {case l(
f + 1 // comment
).u1: break; case l(f+1)
.u1: break; default: break;}