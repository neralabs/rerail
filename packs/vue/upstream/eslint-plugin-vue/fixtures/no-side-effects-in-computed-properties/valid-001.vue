<script>

      Vue.component('test', {
        computed: {
          ...mapGetters(['example']),
          test1() {
            const num = 0
            const something = {
              a: 'val',
              b: ['1', '2']
            }
            num++
            something.a = 'something'
            something.b.reverse()
            return something.b
          }
        }
      })
    
</script>
