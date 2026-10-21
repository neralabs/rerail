test('a', { only: true, only() {} }, () => {
  expect(1).toBe(1);
});
test('b', { skip: true, get skip() { return false; } }, () => {
  expect(1).toBe(1);
});
