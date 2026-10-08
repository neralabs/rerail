var a = 1; // after code
// on its own line
var b = 2; /* after code */
/* before code */ var c = 3;
/* a */ /* b */
function f(/* inside */ x) {
  return x; /* trailing block
  over two lines */
}
/* own
   lines */
var d = [
  // leading
  1, // after an element
  /* before an element */ 2,
];
var e = 1 /* between */ + 2;
var g = 1 +
  /* own line in an expression */
  2;
var h = 1 + /* before a break */
  2;
var i = 1
  /* after a break */ + 2;
