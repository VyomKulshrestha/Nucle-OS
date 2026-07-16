# Vendored from `nanoporetech/minknow_api`

These `.proto` files are copied unmodified from the public
[nanoporetech/minknow_api](https://github.com/nanoporetech/minknow_api)
repository, commit `f8ca84ff1b1f23676cd78e7171d20993e51e225a` (`master`),
under the terms of the Mozilla Public License 2.0 (`LICENSE.txt` in this
directory — see Exhibit A of that license for why the notice lives here
rather than per-file).

Copyright (C) Oxford Nanopore Technologies PLC.

This is the real, public MinKNOW gRPC/protobuf API that Oxford Nanopore
sequencers expose — a *local* instrument-control interface (no cloud
REST API exists for ONT, unlike Twist/IDT/Illumina). `nucle_hardware`'s
`NanoporeProvider` (`../../src/nanopore.rs`) compiles these via
`tonic-build` and speaks the real `ManagerService`/`ProtocolService`
wire protocol. It has not been tested against a live instrument — there
is no ONT hardware in this project's development environment — but it
is a genuine gRPC client against the genuine public spec, not a mock.

Only the subset of files needed for `ManagerService.flow_cell_positions`
and `ProtocolService.start_protocol`/`get_run_info` (and their transitive
message dependencies) is vendored, not the full `proto/` tree — e.g.
`data.proto`, `keystore.proto`, and the `ui/`/`v2/` subdirectories are
unrelated to submitting/monitoring a protocol run and are omitted.
