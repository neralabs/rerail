
it('example', async () => {
  const repqdd = Promise.reject<string>('error');

  await expect(repqdd.then((it) => it.toUpperCase())).rejects.toThrow();
});