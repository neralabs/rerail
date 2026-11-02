
export function a(): void {
  function b() {}
  const x = () => {};
  (function () {});

  function c() {
    return () => {};
  }

  return;
}
      