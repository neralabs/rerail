
        export default {
          computed: {
            foo () {
              return true
            },
            bar: function () {
              return false
            },
            bar3: {
              set () {
                return true
              },
              get () {
                return true
              }
            },
            bar4 () {
              if (foo) {
                return true
              } else {
                return false
              }
            }
          }
        }
      