
function useFoo(){
  const foo = "hi!";
  return useMemo(() => foo, [foo]);
}
