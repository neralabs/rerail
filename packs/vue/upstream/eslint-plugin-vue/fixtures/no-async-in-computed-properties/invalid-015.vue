<script>

      new Vue({
        computed: {
          foo () {
            this.$nextTick(() => {})
            Vue.nextTick(() => {})
            return 'foo'
          }
        }
      })
      
</script>
