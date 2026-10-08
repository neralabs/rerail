
function MyComponent() {
  const pizza = {};

  useEffect(() => ({
    crust: pizza.crust,
    density: pizza?.crust.density,
  }), []);
}
