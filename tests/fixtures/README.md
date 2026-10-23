# Host test fixtures

Use these inputs to test the host's compiler integration, routing and crash recovery.
Keep each fixture tied to the acceptance behaviour it exercises. Runnable applications
and their walkthroughs belong in
[mas-host-examples](https://github.com/emmett08/mas-host-examples).

`go-release` exercises typed generation together with parallel requests and effect
recovery. `rust-invoice` and `cpp-invoice` cover the standalone compiler profiles. The
ordinary CLI `command/tool.py` checks argument and retained-file binding, then exercises
output validation, effect reconciliation and execution limits.
