
function App() {
  const foo = {x: 1};
  const bar = {x: 2};
  useEffect(() => {
    const baz = bar as typeof foo;
    console.log(baz);
  }, []);
}
