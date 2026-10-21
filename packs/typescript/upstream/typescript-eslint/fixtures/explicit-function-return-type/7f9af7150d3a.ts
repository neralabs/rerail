
function fn(arg: boolean) {
  if (arg) {
    return () => (): number => 1;
  } else {
    return function (): string {
      return 'foo';
    };
  }

  return function (): void {};
}
      