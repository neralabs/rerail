
function MyComponent() {
  const pizza = {};

  useEffect(() => ({
    crust: pizza.crust,
    toppings: pizza?.toppings,
  }), []);
}
