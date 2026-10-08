
        <template>
          <template v-for="x in xs">
            <template v-for="y in ys">
              <li v-for="z in zs" :key="x.id + y.id + z.id">
                123
              </li>
            </template>
          </template>
        </template>
      