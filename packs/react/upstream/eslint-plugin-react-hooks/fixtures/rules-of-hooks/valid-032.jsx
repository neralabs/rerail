
function App() {
  const text = use(Promise.resolve('A'));
  return <Text text={text} />
}
