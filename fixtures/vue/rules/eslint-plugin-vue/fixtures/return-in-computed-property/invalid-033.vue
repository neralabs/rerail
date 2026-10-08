<script>

        export default {
          computed: {
            foo () {
              switch (this.type) {
                case 'a':
                  if (this.x) return 1
                case 'b': return 2
              }
            }
          }
        }
      
</script>
