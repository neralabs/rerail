
function foo(key: string | null) {
  const obj = {};
  return obj?.[key!];
}
      