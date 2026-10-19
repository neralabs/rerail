
        export default {
          computed: {
            foo: function () {
              return z.a?.['b'].[c].d.method().catch(err => err).finally(() => {})
            }
          }
        }
      