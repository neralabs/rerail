
        export default {
          computed: {
            foo() {
              return function () {
                return async () => await bar()
              }
            },
          }
        }
      