
        import { cloneElement } from 'react';

        foo.map((item, i) => {
          return cloneElement(someChild, {
            key: i
          })
        })
      
// features: [], parser: default