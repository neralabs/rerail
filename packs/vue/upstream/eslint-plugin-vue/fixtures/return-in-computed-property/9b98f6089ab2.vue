
        <script setup lang="ts">
        import { computed, ref } from 'vue'
        import { Status } from './test01'
        const status = ref<Status>('active')
        const result = computed(() => {
          switch (status.value) {
            case 'active': throw new Error('a')
            case 'inactive': throw new Error('i')
            case 'pending': throw new Error('p')
          }
        })
        </script>