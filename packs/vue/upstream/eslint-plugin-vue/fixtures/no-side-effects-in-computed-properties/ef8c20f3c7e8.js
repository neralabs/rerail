
      Vue.component('test', {
        computed: {
          test () {
            return {
              action1() {
                this.something++
              },
              action2() {
                this.something = 1
              },
              action3() {
                this.something.reverse()
              }
            }
          },
        }
      })
    