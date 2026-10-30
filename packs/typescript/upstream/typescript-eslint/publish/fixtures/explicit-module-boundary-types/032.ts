
export function a(): void {
  function b() {
    function c() {}
  }
  const x = () => {
    return () => 100;
  };
  (function () {
    (function () {});
  });

  function c() {
    return () => {
      (function () {});
    };
  }

  return;
}
      