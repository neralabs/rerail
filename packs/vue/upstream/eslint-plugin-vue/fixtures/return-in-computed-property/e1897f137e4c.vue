
        <script setup lang="ts">
        import { computed, ref } from 'vue'
        import { Status } from './test01'
        const status = ref<Status>('active')
        const result = computed(() => {
          switch (status.value) {
            case 'active': return 1
            case 'inactive': throw new Error('not supported')
            case 'pending': return 3
          }
        })
        </script>