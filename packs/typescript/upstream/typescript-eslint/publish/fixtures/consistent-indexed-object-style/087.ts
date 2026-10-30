
function f(): {
  [k in (keyof ParseResult)]: unknown;
} {
  return {};
}
      