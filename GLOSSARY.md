# specmint

specmint turns an OpenAPI spec into TypeScript files that apps and tests use: types, test-data factories, mock request handlers and validation schemas.

## Language

**Spec**:
The OpenAPI document specmint reads, from a local file or a URL.
_Avoid_: schema (that word means one schema inside the spec), API definition

**Parsed spec**:
specmint's own model of a spec after reading it, which every plugin reads from.
_Avoid_: IR, AST, model

**Plugin**:
A built-in part of specmint that writes one kind of output file from the parsed spec, switched on or off in the config.
_Avoid_: emitter, generator, extension

**Plugin dependency**:
A plugin that another plugin needs switched on, because the second plugin's output imports the first one's.
_Avoid_: requirement, peer

**Factory**:
A generated function that returns realistic fake data for one schema, for use in tests and mocks.
_Avoid_: mock, fixture, builder

**Handler**:
A generated MSW request handler that answers one operation with fake data.
_Avoid_: mock, stub, route
