fn main() {
    // starter:
    //   fork shim, save pid of the shim, exit
    // shim:
    //   setsid 
    //   mark yourself as a subreaper
    //   make pipes for runc stdout/stderr
    //   fork runc
    // runc:
    //   set PDEATH (and check does it still work after exec)
    //   exec `runc create`
    // shim:
    //   read from stdout/stderr & dump to log
    //   dump exit code on runc exit
    println!("Hello, world!");
}
