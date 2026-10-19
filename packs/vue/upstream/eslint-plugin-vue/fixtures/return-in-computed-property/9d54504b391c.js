
        export default {
          computed: {
            foo () {
              switch (this.type) {
                case 'a': return 1
                case 'b': break
                case 'c': return 3
              }
            }
          }
        }
      