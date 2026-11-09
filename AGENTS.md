# Agent instructions

## Builds and test runs

Run every build and test run at a high nice value, so it never starves other work on the
machine:

```
nice -n 19 cargo build …
nice -n 19 cargo test …
nice -n 19 ../target/debug/deps/rerail_engine-… <filter>
```

This holds for background runs too, and for any long command a script starts.
