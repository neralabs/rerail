
        <template>
          <div>
            <div v-text="(prop1?.shift)?.()"></div>
            <div v-text="(this?.prop2)?.shift?.()"></div>
            <div v-text="(this?.prop3?.shift)?.()"></div>
          </div>
        </template>
        <script>
          export default {
            props: ['prop1', 'prop2', 'prop3']
          }
        </script>
      