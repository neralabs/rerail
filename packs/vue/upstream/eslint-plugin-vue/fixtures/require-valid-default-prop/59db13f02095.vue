<script lang="tsx">
export default Vue.extend({
          props: {
            foo: {
              type: Array as PropType<string[]>,
              default: () => []
            }
          }
        });
      
</script>
