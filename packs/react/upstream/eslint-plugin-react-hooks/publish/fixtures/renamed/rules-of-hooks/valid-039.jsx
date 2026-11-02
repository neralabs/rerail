
// Valid because the hook is outside of the loop
const Component = () => {
  const [state, setState] = useState(0);
  for (let e = 0; e < 10; e++) {
    console.log(e);
  }
  return <div></div>;
};
