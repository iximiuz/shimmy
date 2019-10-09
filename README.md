# shimme - container runtime shim

Shimme is a simplistic shim between <a href="https://github.com/iximiuz/conman">_container manager_</a> and <a href="https://github.com/opencontainers/runc">_container runtime_</a>. It does a couple of handy things:

- daemonizes containers being started by runc (i.e. you don't need to use inflexible `runc run --detach`)
- keeps track of the container's exit code
- keeps open PTY for [re-]attaching to container's standard streams

