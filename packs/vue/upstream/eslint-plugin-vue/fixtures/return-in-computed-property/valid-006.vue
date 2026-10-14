<script>

        export default {
          computed: {
            foo () {
              switch (this.type) {
                case 'a': return 1
                case 'b': return 2
                default: return 3
              }
            }
          }
        }
      
</script>
