# Host test fixtures

These inputs exercise compiler integration, routing and crash recovery in this
repository's acceptance suite. Keep changes driven by those tests. Runnable
applications and their walkthroughs belong in
[mas-host-examples](https://github.com/emmett08/mas-host-examples).

`go-release` exercises typed generation, parallel requests and effect recovery.
`rust-invoice` and `cpp-invoice` exercise the standalone compiler profiles.
`command/tool.py` is an ordinary CLI used to check arguments, retained files,
output validation, effect reconciliation and execution limits.
