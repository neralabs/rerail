
        foo.map((item, i) => {
          return React.cloneElement(someChild, {
            key: i
          })
        })
      
// features: [], parser: typescript-eslint