
      Vue.component('test', {
        computed: {
          test () {
            return this.something['a']().reverse()
          },
        }
      })
    