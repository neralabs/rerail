<script>

        export default {
          computed: {
            foo: async function () {
              return await someFunc()
            }
          }
        }
      
</script>
