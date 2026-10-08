<script>
Vue.component('test', {
        computed: {
          test1() {
            return this?.something?.reverse?.()
          },
          test2() {
            return (this?.something)?.reverse?.()
          },
          test3() {
            return (this?.something?.reverse)?.()
          },
        }
      })
</script>
