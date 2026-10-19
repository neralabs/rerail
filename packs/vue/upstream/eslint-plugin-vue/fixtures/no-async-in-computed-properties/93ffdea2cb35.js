
        export default {
          computed: {
            foo: function () {
              return someLib.string().catch(42)
            }
          }
        }
      