<script>

        export default {
          computed: {
            foo: function () {
              return z.number().catch(42)
            }
          }
        }
      
</script>
