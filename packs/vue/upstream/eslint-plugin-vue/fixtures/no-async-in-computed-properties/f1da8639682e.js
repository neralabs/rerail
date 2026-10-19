
      new Vue({
        computed: {
          async foo () {
            await this.$nextTick()
            await Vue.nextTick()
            return 'foo'
          }
        }
      })
      