# Usage

`watchexec-simple` is a simpler alternative for the existing `watchexec` project. It doesn't implement nearly 
all the features that the full `watchexec` project provides. However, it provides the core functionality in a codebase
that is 2% the size of `watchexec` project. It is built on the same `notify` library, itself built by the same authors as `watchexec`. 
`watchexec-simple` installs to the same binary name as the `watchexec` project.

Here is a simple example of using `watchexec-simple`:

    watchexec -- cargo run

### Comparison to `watchexec`

When possible, `watchexec-simple` relies on the same option names as `watchexec`. The key differences are:

1. For `watchexec-simple`, positional arguments are watched paths, and `--` is required and used to separate the command. For `watchexec`, positional
   arguments are the command, and each path requires a `-w` to be passed in. For example:
   
```bash
# watchexec-simple
watchexec src/ data/ .env -- cargo run

# watchexec
watchexec -w src/ -w data/ -w .env cargo run
```

2. By default, `watchexec-simple` restarts the process, even if it is actively running. For `watchexec`, the user is required to pass the `-r` option. Example:

```bash
# watchexec-simple
watchexec -- cargo run

# watchexec
watchexec -r cargo run
```

# Installation

Not currently published to cargo. Git clone it and then install from local.

    git clone https://github.com/kurtbuilds/watchexec-simple
    cd watchexec-simple
    cargo install --path .
