
        import foo from 'foo';
        function innerContext() {
          return (
            <Context.Provider value={foo.something}></Context.Provider>
          )
        }
      