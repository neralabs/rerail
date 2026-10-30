
function wrap() {
  function foo(s: string);
  function foo(n: number);
  type bar = number;
  function foo(sn: string | number) {}
}
      