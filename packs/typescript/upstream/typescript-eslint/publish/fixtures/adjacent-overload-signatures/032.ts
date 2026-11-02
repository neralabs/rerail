
function wrap() {
  function foo(s: string);
  function foo(n: number);
  function foo(sn: string | number) {}
}
    