
function foo(bar: { (s: string): number } | undefined): number {
  return bar('hello');
}
      