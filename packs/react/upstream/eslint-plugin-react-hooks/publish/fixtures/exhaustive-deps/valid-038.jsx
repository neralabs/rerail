
const local = {};
function MyComponent() {
  const myEffect = () => {
    otherThing()
  }
  const otherThing = () => {
    console.log(local);
  }
  useEffect(myEffect, []);
}
