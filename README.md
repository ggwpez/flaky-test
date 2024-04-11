# Cargo flaky-test

Tries to make a flaky Rust test fail or time out and dumps the trace of all threads to disk. Assumes `lldb` to be installed.

**Install:**

```sh
cargo install --git https://github.com/ggwpez/flaky-test
```

## Example

Investigating the `voter_persists_its_votes` test of the `sc-consensus-grandpa` crate in the `polkadot-sdk` workspace:

```sh
RUST_LOG=info flaky-test \
	--manifest-path ../polkadot-sdk \
	-p sc-consensus-grandpa voter_persists_its_votes \
	--timeout 120 --batch 100 --reps 10
```

Shows us that it indeed fails:

![Example](./.assets/example.png)

Now you can take a look at the stacktrace file, or start a debugger and attach it to the printed PID.
