
      Vue.component('test', {
        computed: {
          test () {
            return [...this.items].reverse()
          },
        }
      })
    