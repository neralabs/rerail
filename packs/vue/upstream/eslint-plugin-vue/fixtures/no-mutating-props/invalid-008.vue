
        <template>
          <div>
            <MyComponent :data.sync="this.prop" />
            <MyComponent :data.sync="prop" />
            <MyComponent :data="this.prop" />
            <MyComponent :data="prop" />
            <template v-for="prop of data">
              <MyComponent :data.sync="prop" />
              <MyComponent :data.sync="this.prop" />
            </template>
          </div>
        </template>
        <script>
          export default {
            props: ['prop']
          }
        </script>
      