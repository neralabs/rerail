<script lang="ts">

        export default Vue.extend({
          computed: {
            test1() : string {
              return this.something.reverse()
            }
          }
        });
      
</script>
