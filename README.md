# shimmy - container runtime shim

Shimmy is a simplistic shim between <a href="https://github.com/iximiuz/conman">_container manager_</a> and <a href="https://github.com/opencontainers/runc">_container runtime_</a>. It's primary designed to make programmatic _runc_ execution more friendly for the launching process. It does a couple of handy things:

- Detaches container runtime process from the launching process.
- Forwards container STDOUT and STDERR to logs.
- Tracks container termination and writes its status on disk.
- [TODO] Allows attaching to container STDIN to forward some data in.
- [TODO] Allows attaching to container STDOUT & STDERR to read some data from.
- [TODO] PTY-driven attaching.

Similar projects:

- <a href="https://github.com/containers/conmon">conmon</a>
- <a href="https://github.com/containerd/containerd/blob/master/runtime/v2/shim.go">containerd runtime shim</a>

Read more about the project on my blog:

- <a href="https://iximiuz.com/en/posts/implementing-container-runtime-shim">Implementing container runtime shim: runc</a>
- <a href="https://iximiuz.com/en/posts/implementing-container-runtime-shim-2">Implementing container runtime shim: first code</a>

## Usage

```bash
# build debug version
cargo build --bin shimmy

# build release version
cargo build --bin shimmy --release

# integrational tests (conman is required)
https://github.com/iximiuz/conman/blob/v0.0.2/test/shimmy/shimmy_test.go
```

