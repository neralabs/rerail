
function MyComponent() {
  const local = someFunc();
  function myEffect() {
    console.log(local);
  }
  useEffect(myEffect, [local]);
}
