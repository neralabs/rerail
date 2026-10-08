
        <template>
          <input v-model="foo">
          <input v-model="bar">
          <input v-model="window">
          <input v-model="Infinity">
        </template>
        <script setup>
        import { ref } from 'vue'
        const { Infinity, obj } = defineProps({
          foo: String,
          bar: String,
          Infinity: String,
          window: String,
          obj: Object
        })
        const foo = ref('')
        obj.id = 2
        </script>
      