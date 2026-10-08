<script setup lang="ts">
      export interface SomePropInterface {
        someProp?: false | string;
        str?: 'foo' | 'bar';
        num?: 1 | 2;
      }

      withDefaults(defineProps<SomePropInterface>(), {
        someProp: false,
        str: 'foo',
        num: 1
      });
      </script>