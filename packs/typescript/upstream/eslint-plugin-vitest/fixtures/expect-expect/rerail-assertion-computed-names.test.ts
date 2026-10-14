it('string key', () => {
  assert['ok'](true);
});
it('identifier key', () => {
  assert[method](true);
});
it('parenthesized helper', (check));

function check() {
  expect(1).toBe(1);
}
