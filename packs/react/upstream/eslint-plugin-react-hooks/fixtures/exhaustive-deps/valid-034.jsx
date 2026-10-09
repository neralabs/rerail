
const local = {};
function MyComponent() {
  const myEffect = () => {
    console.log(local);
  };
  useEffect(myEffect, []);
}
