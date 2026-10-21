
        foo.map((item, i) => {
          return React.cloneElement(someChild, {
            key: item.id
          })
        })
      
// features: [], parser: @typescript-eslint/parser