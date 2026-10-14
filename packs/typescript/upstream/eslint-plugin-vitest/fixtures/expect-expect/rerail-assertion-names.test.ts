it('parenthesized', check);
it('case', () => {
  Expect(1).toBe(1);
});
it('tagged template', () => {
  expect`x`;
});

function check() {
  (expect)(1).toBe(1);
}
