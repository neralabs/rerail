
function MyComponent() {
  const local = {};
  const myEffect = () => {
    otherThing();
  };
  const otherThing = () => {
    console.log(local);
  };
  useEffect(myEffect, []);
}
