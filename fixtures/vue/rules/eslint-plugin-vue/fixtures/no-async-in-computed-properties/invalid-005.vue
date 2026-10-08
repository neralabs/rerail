<script>

        export default {
          computed: {
            foo: function () {
              return bar.catch(e => {})
            }
          }
        }
      
</script>
