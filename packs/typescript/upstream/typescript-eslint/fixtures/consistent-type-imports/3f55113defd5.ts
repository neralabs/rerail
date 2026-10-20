
import foo from 'foo';
type Baz = (typeof foo.bar)['Baz']; // TSQualifiedName & TSTypeQuery
          