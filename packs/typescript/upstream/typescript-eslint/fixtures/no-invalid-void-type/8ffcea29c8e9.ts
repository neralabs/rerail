
async function foo(bar: () => void | Promise<void>) {
  await bar();
}
      