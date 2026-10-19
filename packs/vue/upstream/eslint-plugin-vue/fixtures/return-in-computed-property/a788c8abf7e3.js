
        export default {
          computed: {
            foo: function () {
              function bar () {
                return this.baz * 2
              }
              bar()
            }
          }
        }
      