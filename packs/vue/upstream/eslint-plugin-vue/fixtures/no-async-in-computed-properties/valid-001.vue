<script>

        export default {
          ...foo,
          computed: {
            ...mapGetters({
              test: 'getTest'
            }),
            foo: function () {
              var bar = 0
              try {
                bar = bar / 0
              } catch (e) {
                return e
              } finally {
                return bar
              }
            },
            bar: {
              set () {
                new Promise((resolve, reject) => {})
              }
            },
            baz: {
              ...mapGetters({ get: 'getBaz' }),
              ...mapActions({ set: 'setBaz' })
            }
          }
        }
      
</script>
