foo(/* inline */ a);
foo(a, /* not inline */
  b);
foo(a,
  /* not inline */ b);
foo(a,
  /* not inline */
  b);
/* first in the file? no */ foo(a);
foo(a); /* last on its line */
var x = /* inline too */ 1;
var y = [/* two */ /* comments */ 1];
// a line comment
foo(a); // a trailing line comment
