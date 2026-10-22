
      <script setup>
      const props = defineProps({ count: Number })
      const count = computed(() => props.count)
      </script>
      