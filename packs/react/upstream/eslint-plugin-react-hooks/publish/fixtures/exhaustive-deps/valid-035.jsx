
const local = {};
function MyComponent() {
  function myEffect() {
    console.log(local);
  }
  useEffect(myEffect, []);
}
