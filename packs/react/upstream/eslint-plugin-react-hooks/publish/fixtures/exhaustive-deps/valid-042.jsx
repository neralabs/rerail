
let local = {};
function myEffect() {
  console.log(local);
}
function MyComponent() {
  useEffect(myEffect, []);
}
