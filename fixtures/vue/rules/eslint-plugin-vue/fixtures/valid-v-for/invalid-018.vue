
        <template>
          <template v-for="x in xs">
            <template v-for="y in a.ys">
              <li v-for="z in y.zs" :key="z.id">
                123
              </li>
            </template>
          </template>
        </template>
      