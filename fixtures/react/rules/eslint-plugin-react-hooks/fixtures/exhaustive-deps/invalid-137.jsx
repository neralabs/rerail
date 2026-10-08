
function MyComponent() {
  const local = {};
  function myEffect() {
    console.log(local);
  }
  useEffect(myEffect, []);
}
