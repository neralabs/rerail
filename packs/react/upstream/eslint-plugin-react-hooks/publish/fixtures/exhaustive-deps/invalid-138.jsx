
function MyComponent() {
  const local = {};
  const myEffect = () => {
    console.log(local);
  };
  useEffect(myEffect, []);
}
