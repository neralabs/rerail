
function MyComponent(props) {
  let value;
  let value2;
  let value3;
  let asyncValue;
  useEffect(() => {
    value = {};
    value2 = 100;
    value = 43;
    console.log(value2);
    console.log(value3);
    setTimeout(() => {
      asyncValue = 100;
    });
  }, [value, value2, value3]);
}
