
function fn(arg: boolean) {
  if (arg) return 'string';
  return function (): void {};
}
      