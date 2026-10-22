<script>

        export default {
          computed: {
            foo: function () {
              return z.catch(
                z.string().check(z.minLength(2)),
                'default'
              ).then(val => val).finally(() => {})
            }
          }
        }
      
</script>
