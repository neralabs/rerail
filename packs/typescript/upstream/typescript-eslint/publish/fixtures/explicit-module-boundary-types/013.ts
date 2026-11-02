
export function test(): string {
  const nested = () => 'value';
  return nested();
}
      