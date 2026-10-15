
        const baz = foo?.bar?.()?.[1] ?? 'qux';

        qux()?.map()

        const directiveRanges = comments?.map(tryParseTSDirective)
      
// features: [optional chaining,nullish coalescing], parser: default, parserOptions: {"ecmaVersion":2020}