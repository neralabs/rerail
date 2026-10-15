<script>

      export default {
        computed: {
          foo: function () {
            return Promise.reject([])
          }
        }
      }
      
</script>
