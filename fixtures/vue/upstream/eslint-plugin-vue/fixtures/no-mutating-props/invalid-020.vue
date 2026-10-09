
        <script setup lang="ts">
        const props = withDefaults(defineProps<Props>(), {
          msg: 'hello'
        })
        props.value++
        </script>
      