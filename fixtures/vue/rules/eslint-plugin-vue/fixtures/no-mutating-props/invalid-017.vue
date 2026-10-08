
        <template>
          <input v-model="value">
          <input v-model="props.value">
        </template>
        <script setup>
        const props = defineProps({
          value: String,
        })
        </script>
      