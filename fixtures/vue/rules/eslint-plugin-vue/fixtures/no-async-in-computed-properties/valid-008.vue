<script>

        export default {
          computed: {
            foo() {
              return someFunc.doSomething({
                async bar() {
                  return await baz()
                }
              })
            },
          }
        }
      
</script>
