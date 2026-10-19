
        export default {
          computed: {
            foo () {
              switch (this.type) {
                case 'a': return 1
                case 'b': console.log('b')
                case 'c': return 3
              }
            }
          }
        }
      