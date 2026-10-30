describe.notVitest.only('suite', () => {});
test.notVitest('focused', { only: true }, () => {
  expect(1).toBe(1);
});
it.only.only('twice', () => {});
