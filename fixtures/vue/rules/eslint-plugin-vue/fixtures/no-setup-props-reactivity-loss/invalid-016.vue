
      <script setup>
      const props = defineProps({ count: Number })
      const count = ref(props.count)
      count = fn(props.count)
      </script>
      