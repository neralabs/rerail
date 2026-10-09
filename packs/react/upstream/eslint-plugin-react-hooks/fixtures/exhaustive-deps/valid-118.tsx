
function App() {
  const foo = {x: 1};
  React.useEffect(() => {
    const bar = {x: 2};
    const baz = bar as typeof foo;
    console.log(baz);
  }, []);
}
