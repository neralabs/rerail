
      <script setup>
      import {toRefs} from 'vue'
      const props = defineProps(['foo', 'bar'])
      const { foo, bar } = toRefs(props)
      </script>
      