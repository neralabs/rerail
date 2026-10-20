
function foo(n) {
  if (n < 2) return 1;
  return n * foo(n - 1);
}
      