
function f(): {
  // intentionally not using a Record to preserve optionals
  [k in keyof ParseResult]: unknown;
} {
  return {};
}
      