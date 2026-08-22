oslo.direnv.nix_develop()
oslo.direnv.path_add("./target/debug")
oslo.direnv.path_add("./bin")
oslo.direnv.path_add("./target/x86_64-unknown-linux-musl/release")

oslo.env.set("TOP_HEAD", oslo.sys.pwd())

oslo.env.unset("GITHUB_TOKEN")

oslo.env.set_alias("_b", "make build")
oslo.env.set_alias("_c", "make check")
oslo.env.set_alias("_r", "make run")
oslo.env.set_alias("_t", "make test")
oslo.env.set_alias("_v", "make verify")
oslo.env.set_alias("_i", "make install")
