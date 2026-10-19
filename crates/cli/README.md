# Cli

## Testing

You can test your changes to the `cli` crate by first building the main fanta binary:

```
cargo build -p fanta
```

And then building and running the `cli` crate with the following parameters:

```
 cargo run -p cli -- --fanta ./target/debug/fanta.exe
```
