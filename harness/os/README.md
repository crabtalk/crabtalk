# berm-os

`bash`, `read`, `edit`, `glob` and `grep` as a harness. Grants `fs` and `exec`,
both bounded by `root`. Searching honours `.gitignore`.

```sh
cargo test -p berm-os    # tools run natively, no RISC-V toolchain
make harness             # build and install to ~/.crabtalk/harnesses
```

## License

Apache-2.0
