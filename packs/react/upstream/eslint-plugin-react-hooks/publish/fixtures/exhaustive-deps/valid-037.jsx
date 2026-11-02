
function MyComponent() {
  function myEffect() {
    console.log(global);
  }
  useEffect(myEffect, []);
}
