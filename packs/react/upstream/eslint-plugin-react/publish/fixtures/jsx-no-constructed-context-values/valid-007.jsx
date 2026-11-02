
        // Passes because the lint rule doesn't handle JSX spread attributes
        function innerContext() {
          const foo = {value: 'something'}
          return (
            <Context.Provider {...foo}></Context.Provider>
          )
        }
      