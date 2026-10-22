
        <script setup lang="ts">
        import { computed, ref } from 'vue'
        import { LiteralsWithObject } from './test01'
        const val = ref<LiteralsWithObject>('a')
        const result = computed(() => {
          switch (val.value) {
            case 'a': return 1
            case 'b': return 2
          }
        })
        </script>