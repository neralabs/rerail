
it('example', async () => {
  const result = Promise.reject<string>('error');

  await expect(result.then((it) => it.toUpperCase())).rejects.toThrow();
});