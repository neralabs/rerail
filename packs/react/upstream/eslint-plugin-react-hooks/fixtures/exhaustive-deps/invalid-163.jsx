
function Foo(section) {
  const foo = section.section_components?.edges ?? [];
  useEffect(() => {
    console.log(foo);
  }, [foo]);
}
