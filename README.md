# shimmy - container runtime shim

Shimmy is a simplistic shim between <a href="https://github.com/iximiuz/conman">_container manager_</a> and <a href="https://github.com/opencontainers/runc">_container runtime_</a>. It does a couple of handy things:

- daemonizes containers being started by runc (i.e. you don't need to use inflexible `runc run --detach`)
- keeps open PTY for [re-]attaching to container's standard streams
- keeps track of the container's exit code

Similar projects:

- <a href="https://github.com/containers/conmon">conmon</a>
- <a href="https://github.com/containerd/containerd/blob/master/runtime/v2/shim.go">containerd runtime shim</a>

## TODO:
- Keep the state of the program in a struct instead of global vars.
- Implement the shim as FSM by defining each state and transitions.
- git diff 8392df88fba944510b51c7d5b92aa745a15863f8..HEAD

