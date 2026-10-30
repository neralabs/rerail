
        // Passes because the lint rule doesn't handle JSX spread attributes
        function innerContext() {
          const foo = useMemo(() => {
            return bar;
          })
          return (
            <Context.Provider value={foo}></Context.Provider>
          )
        }
      