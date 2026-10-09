<T> ({});

({}) as T;

<T> x;

x as T;

// Allow cast to 'any'
{} as any;
<any> {};

// Allow cast to 'unknown'
{} as unknown;
<unknown> {};

// Allow const assertion
({}) as const;
<const> ({});

foo({} as T);

foo(<T>{});

