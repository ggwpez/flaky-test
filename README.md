# Cargo flaky-test

Tries to make a flaky Rust test fail or time out and dumps the trace of all threads to disk. Assumes `lldb` to be installed.

## Example

```sh
RUST_LOG=info flaky-test --manifest-path ../polkadot-sdk -p sc-consensus-grandpa voter_persists_its_votes --timeout 120 --batch 100 --reps 10
```

![Example](./.assets/example.png)
