<script>

      export default {
        computed: {
          foo: function () {
            return Promise.race([])
          }
        }
      }
      
</script>
