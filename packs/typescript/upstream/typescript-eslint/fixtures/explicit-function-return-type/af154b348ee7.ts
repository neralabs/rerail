
function fn() {
  const bar = () => (): number => 1;
  return function (): void {};
}
      