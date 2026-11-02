
function ComponentUsingFormState(props) {
  const [state7, dispatch3] = useFormState();
  const [state8, dispatch4] = ReactDOM.useFormState();
  useEffect(() => {
    dispatch3();
    dispatch4();

    // dynamic
    console.log(state7);
    console.log(state8);

  }, [state7, state8]);
}
