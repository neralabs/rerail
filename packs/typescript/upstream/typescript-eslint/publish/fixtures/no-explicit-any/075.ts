
const integer = <
  TKey extends keyof any,
  TTarget extends { [K in TKey]: number },
>(
  target: TTarget,
  key: TKey,
) => {
  /* ... */
};
      