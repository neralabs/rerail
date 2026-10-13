
function foo(bar: { (s: string): number }): number {
  return bar('hello');
}
      