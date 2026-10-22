# C++ application SDK

The initial Linux/C++17 profile exposes an application function:

```cpp
#include "mas.hpp"

mas::Json Assess(mas::Context&, const mas::Json& input) {
    return {{"approved", input.at("amount_minor") == 12500}};
}
```

An agent's `source` names the C++ source file and `handler` names `Assess`.
`mas-host build` generates `main`, includes the source and SDK, and invokes the
system `c++` compiler. Application code contains no launcher or transport code.
This first build profile supports standard-library code, the SDK and application
headers included from the source. Custom linker flags, external libraries,
arbitrary build systems and automatic schema extraction are future profiles.

`Context::request(key, target, input)` returns a durable child result.
`Context::parallel({{key,target,input}, ...})` preserves request order. A context
is non-copyable and serialises channel commands. Application threads must finish
before their handler returns; they do not become durable workflow steps.
`Context::effect(key, descriptor, callback)` supplies a stable admitted identity
to the callback and expects `std::pair<mas::Json,mas::Json>{output,evidence}`.
Cached effects skip the callback. Failure before acknowledged completion requires
evidence-based resolution. The host owns admission, deadlines,
durable state and recovery.

A governed completion returns the host's safe projection, matching cached
replay. Withheld payloads return the reserved `mas-governed-unavailable/1` object
while host settlement remains completed. Malformed replacement metadata throws
`std::runtime_error("invalid governed effect completion")` without returning local
raw output or retrying. An acknowledgement without `completion` preserves the
ordinary callback output. See the closed versioned acknowledgement, effect
identity, external certainty and payload disposition in the
[native contract](../../docs/native-contract.md#external-effects-and-recovery).

`mas::Json` is the vendored nlohmann/json 3.12.0 type. Inputs, outputs,
descriptors and evidence are objects. The SDK rejects duplicate keys (including
escaped aliases), invalid Unicode, floats, excessive nesting and frames above
65,536 bytes. **This C++ profile supports signed and unsigned 64-bit integers.**
Larger JSON integers are rejected explicitly instead of rounded. Use decimal
strings when the application needs larger integers, and scaled integers for
money. This numeric subset is narrower than the MAS protocol's exact-integer
domain and the Rust SDK; do not send larger numeric values to a C++ handler.

Application code is trusted, not sandboxed. The private channel, process lifetime
and worker `main` are supplied by MAS. Ordinary code outside effect boundaries
must be deterministic and free of external effects if it may be replayed.

Run SDK conformance tests from the repository root:

```sh
c++ -std=c++17 -Wall -Wextra -Werror -pthread -Isdk/cpp/include \
  sdk/cpp/tests/conformance.cpp -o /tmp/mas-cpp-conformance
/tmp/mas-cpp-conformance
```

The tests cover strict JSON, Unicode, integer range, depth, frame exchange,
ordered parallel results, cached effects and admitted-effect evidence.

Vendored dependency provenance: `include/nlohmann/json.hpp` is the unmodified
single header from `https://github.com/nlohmann/json`, tag `v3.12.0`, path
`single_include/nlohmann/json.hpp`. Its MIT licence is retained beside it in
`LICENSE.MIT`. The MAS SDK itself follows the repository's GPL-3.0-only licence.
