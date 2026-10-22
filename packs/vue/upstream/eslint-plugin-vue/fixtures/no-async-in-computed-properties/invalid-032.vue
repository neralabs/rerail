<script>

        export default {
          computed: {
            foo: function () {
              return myFunc().catch('default')
            }
          }
        }
      
</script>
