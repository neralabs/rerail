
        function Foo(section) {
          const foo = section.section_components?.edges;

          return (
            <Context.Provider value={foo}></Context.Provider>
          )
        }
      