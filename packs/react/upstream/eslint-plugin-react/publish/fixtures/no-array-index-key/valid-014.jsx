
        foo.map((item, i) => {
          return cloneElement(someChild, {
            key: item.id
          })
        })
      