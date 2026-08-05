# Sigrok Decoder Distribution

## Native application policy

LogicConduit hosts API-version-3 Sigrok Python decoder packages supplied through explicit decoder
search paths. Decoder packages are not embedded in the native application bundle. The catalog
shows each package's declared license beside its decoder name and preserves discovery failures as
diagnostics rather than silently omitting them.

Decoder scripts are trusted executable code. Selecting a search path authorizes Python packages
under that directory to run with the native application's permissions when they are discovered or
used. Search-path order is significant: the first successfully discovered package for a decoder
ID wins, and later duplicates are reported.

## Packaging boundary

The native package contains the PyO3 host and CPython integration, but no `libsigrokdecode` C
library and no third-party decoder collection. Packaging therefore does not combine LogicConduit's
MIT-licensed Rust code with decoder-package licenses.

The wasm application does not register the native Sigrok decoder node or CPython host.

## Validation

Catalog tests use injected search-path and package-discovery implementations to cover ordered
paths, duplicate IDs, missing and unreadable directories, invalid packages, cache reuse, and
explicit refresh without executing Python. Separate project-owned decoder fixtures cover Python
metadata including licenses and native-adapter behavior. Runtime-node tests inject the Rust-owned
execution contract and do not load a Python package. Generic UI, compiler, viewer, and node-graph
architecture tests reject Sigrok-specific host cases.
