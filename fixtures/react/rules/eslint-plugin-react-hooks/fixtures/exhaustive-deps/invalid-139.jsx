
function MyComponent() {
  const local = {};
  const myEffect = function() {
    console.log(local);
  };
  useEffect(myEffect, []);
}
