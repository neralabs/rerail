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

// Allow object literals to be asserted when used as arguments.
foo({} as T);
foo(<T>{});

