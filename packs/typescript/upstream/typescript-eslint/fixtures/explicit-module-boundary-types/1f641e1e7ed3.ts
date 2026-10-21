
export function a() {
  return function b(): () => void {
    return function c() {};
  };
}
      