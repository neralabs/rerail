
        async function resolveComponents(components) {
          return await Promise.all(components.map(async (component) => {
              if(typeof component === 'function') {
                    return await component()
                }
                return component;
          }));
        }
      