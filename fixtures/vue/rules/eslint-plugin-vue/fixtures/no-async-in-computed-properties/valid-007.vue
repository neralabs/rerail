<script>

        export default {
          computed: {
            foo() {
              return new Bar(async () => await baz())
            },
          }
        }
      
</script>
