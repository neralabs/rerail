
        export default {
          computed: {
            foo() {
              const a = 'test'
              return [
                async () => {
                  const baz = await bar(a)
                  return baz
                },
                'b',
                {}
              ]
            }
          }
        }
      