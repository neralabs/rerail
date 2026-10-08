<script lang="ts">

        export default (Vue as VueConstructor<Vue>).extend({
          props: {
            foo: {
              type: [Object, Number],
              default: 10
            } as PropOptions<object>
          }
        });
      
</script>
