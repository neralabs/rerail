<script>

        export default {
          computed: {
            foo: function () {
              return (bar?.then)?.(response => {})
            }
          }
        }
      
</script>
