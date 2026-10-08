
        <template>
          <input v-model="foo">
          <input v-model="bar">
          <input v-model="Infinity">
        </template>
        <script setup>
        import { ref } from 'vue'
        import { bar } from './my-script'
        defineProps({
          foo: String,
          bar: String,
          Infinity: Number
        })
        const foo = ref('')
        </script>
      