# Usage

`watchexec-simple` is a simpler alternative for the existing `watchexec` project. It doesn't implement nearly 
all the features that the `watchexec` project provides. However, it provides the core functionality in a codebase
that is 2% the size of `watchexec` project. It is still built on `notify`, built by the same authors as `watchexec`.

Here is a simple example:

    watchexec -- just run

The `--` separating the command is required.

# Installation

Not currently published to cargo. Git clone it and then install from local.

    git clone https://github.com/kurtbuilds/watchexec-simple
    cd watchexec-simple
    cargo install --path .

# Examples

    watchexec -- just run
