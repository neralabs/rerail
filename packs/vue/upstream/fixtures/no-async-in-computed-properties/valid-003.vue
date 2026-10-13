<script>

        export default {
          computed: {
            foo() {
              return {
                async bar() {
                  const data = await baz(this.a)
                  return data
                }
              }
            }
          }
        }
      
</script>
