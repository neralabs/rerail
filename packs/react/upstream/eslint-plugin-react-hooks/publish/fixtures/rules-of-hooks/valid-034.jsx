
function App() {
  let data = [];
  for (const query of queries) {
    const text = use(item);
    data.push(text);
  }
  return <Child data={data} />
}
