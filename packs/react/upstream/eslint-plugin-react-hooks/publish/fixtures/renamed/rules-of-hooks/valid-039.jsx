
// Valid because the hook is outside of the loop
const Component = () => {
  const [state, setState] = useState(0);
  for (let j = 0; j < 60; j++) {
    console.log(j);
  }
  return <div></div>;
};
