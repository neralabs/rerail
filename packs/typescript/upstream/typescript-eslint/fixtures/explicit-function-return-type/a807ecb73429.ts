
function fn() {
  const bar = () => (): number => 1;
  const baz = () => () => 'baz';
  return function (): void {};
}
      