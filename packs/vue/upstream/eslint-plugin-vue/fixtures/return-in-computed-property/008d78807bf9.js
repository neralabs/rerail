
        export default {
          computed: {
            foo () {
              switch (this.type) {
                case 'a': return 1
                case 'b': return 2
                case 'c': return 3
              }
            }
          }
        }
      